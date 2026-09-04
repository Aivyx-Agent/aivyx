# Daemon-Side Automatic Alert Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a tool process push a notification through the daemon without any daemon-initiated invocation, and wire `aivyx-toolkit`'s health-check watcher up to it, closing a real gap that's been sitting undone since Phase 125 (65 phases ago).

**Architecture:** A new call_id-free `ToolToDaemon::DispatchNotification` wire variant flows from a tool process's background task, through a new optional sink hook on `ToolProcessBridge` (kept capability-agnostic — `aivyx-tool` never learns about scopes or `NotifyDispatcher`), to a capability-checked adapter constructed in the daemon binary that calls the real `NotifyDispatcher`. On the tool-process side, `aivyx-toolkit`'s health-check polling loop (which runs independently of any daemon-initiated tool call) gets a new outbound channel multiplexed into the harness's existing stdout writer, since two independent writers on one stdout stream would corrupt each other's frames.

**Tech Stack:** Rust, tokio (mpsc channel, background tasks), existing `aivyx-tool`/`aivyx-capability`/`aivyx-channel`/`aivyx-toolkit` crates — no new dependencies.

## Global Constraints

- New wire variant: `ToolToDaemon::DispatchNotification { target: String, message: String, subject: Option<String> }` in `crates/aivyx-tool/src/wire.rs` — no `call_id` field (it isn't a response to any `InvokeTool`).
- `aivyx-tool` MUST NOT gain a dependency on `aivyx-channel` or `aivyx-capability` — verified today via every crate's `Cargo.toml` that neither depends on the other, and several unrelated tool-process crates (`aivyx-gmail`, `aivyx-calendar`, `aivyx-drive`, `aivyx-obsidian`, `aivyx-notion`, `aivyx-n8n`, `aivyx-contacts`, `aivyx-apps`, `aivyx-vertical-sdk`) depend on `aivyx-tool` alone and have no business pulling in the full channel/capability stack.
- New capability scope: `notify.dispatch`, added to `aivyx-capability`'s `KNOWN_BASES`. The check is `CapabilitySet::grants(&Scope) -> bool` (`crates/aivyx-capability/src/lib.rs:868`) — `capabilities.grants(&Scope::parse("notify.dispatch").unwrap())`. This is D4's *only* authoritative grant check; never compare scope strings ad hoc.
- Both transition directions (down and recovered) trigger a notification — matches the existing agent-mediated recipe's own behavior (`docs/INSTALL.md:4426`), not a behavior change.
- Target selection: one `[toolkit] default_notify_target` config field (`~/.aivyx/tool-processes/toolkit/config.toml`) — no per-watcher targets in this plan.
- Error handling: unconfigured target → skip silently (log, no dispatch attempt). Unknown target or backend send failure → log via `ToolEventPayload::Log`, never panic, never abort the polling loop.
- `health_store.rs`'s `record_check` already detects real transitions only (`had_prior_check && prev != outcome.ok`) — this plan changes its return type to surface that detection, not its detection logic.
- Out of scope: per-watcher targets, any tool process other than `aivyx-toolkit` actually using the new hook, rate-limiting beyond what transition-only detection already gives, the unrelated `run_multi_tool_subprocess`-harness-lift tech debt from Phase 123.

---

### Task 1: Wire protocol — `DispatchNotification`

**Files:**
- Modify: `crates/aivyx-tool/src/wire.rs`

**Interfaces:**
- Produces: `ToolToDaemon::DispatchNotification { target: String, message: String, subject: Option<String> }` — the exact variant every later task sends/receives.

- [ ] **Step 1: Add the variant**

In `crates/aivyx-tool/src/wire.rs`, find the `ToolToDaemon` enum (starts at the `#[serde(tag = "type")]` above `pub enum ToolToDaemon {`) and add a new variant after `ToolError`:

```rust
    /// Phase 191 — a tool process pushes a notification with no
    /// preceding `InvokeTool` (no `call_id`: this isn't a response
    /// to anything, it fires from the tool process's own background
    /// task, e.g. a health-check watcher's polling loop noticing a
    /// state change). The daemon capability-checks the sending tool
    /// process before honoring this — see `aivyx-tool`'s
    /// `NotificationSink` trait (Task 2) for the injection point;
    /// `aivyx-tool` itself has no opinion on scopes.
    DispatchNotification {
        target: String,
        message: String,
        subject: Option<String>,
    },
```

- [ ] **Step 2: Write the round-trip test**

In the `#[cfg(test)] mod tests` block at the bottom of the file, add a new test (near `tool_to_daemon_round_trips`):

```rust
    #[test]
    fn dispatch_notification_round_trips() {
        roundtrip(&ToolToDaemon::DispatchNotification {
            target: "phone".into(),
            message: "watcher x went down".into(),
            subject: Some("Health alert".into()),
        });
        roundtrip(&ToolToDaemon::DispatchNotification {
            target: "phone".into(),
            message: "watcher x recovered".into(),
            subject: None,
        });
    }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p aivyx-tool dispatch_notification_round_trips -- --nocapture`
Expected: `test wire::tests::dispatch_notification_round_trips ... ok`

- [ ] **Step 4: Run the full crate suite to confirm no regression**

Run: `cargo test -p aivyx-tool`
Expected: all existing tests still pass, plus the new one.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-tool/src/wire.rs
git commit -m "feat(tool): add DispatchNotification wire variant — Phase 191"
```

---

### Task 2: Bridge routing — `NotificationSink` + `ToolProcessBridge` wiring

**Files:**
- Modify: `crates/aivyx-tool/src/bridge.rs`

**Interfaces:**
- Consumes: `ToolToDaemon::DispatchNotification` from Task 1.
- Produces: `pub trait NotificationSink: Send + Sync { fn dispatch(&self, target: String, message: String, subject: Option<String>); }`; a new `notification_sink: Option<Arc<dyn NotificationSink>>` field on `ToolProcessConfig`. Task 3 implements this trait against the real `NotifyDispatcher` and sets this field when constructing the toolkit's `ToolProcessConfig`.

Confirmed against the real file (`crates/aivyx-tool/src/bridge.rs`): the constructor is `ToolProcessBridge::spawn(config: ToolProcessConfig) -> Result<Self, ToolBridgeError>` (line 147) — a single config struct, not a long parameter list. `ToolProcessConfig` (line 78-92) currently has `name`, `command`, `args`, `env`, `sandbox: Option<SandboxConfig>`, and derives `#[derive(Debug, Clone)]`. `reader_loop` (line 353) currently takes exactly 2 parameters: `reader: BufReader<ChildStdout>` and `pending: Arc<Mutex<HashMap<...>>>`, called from `spawn` at line 224-226 as `reader_loop(reader, pending_for_reader).await` inside a `tokio::spawn`.

- [ ] **Step 1: Add the `NotificationSink` trait**

Near the top of `crates/aivyx-tool/src/bridge.rs`, after the existing `use` statements, add:

```rust
/// Phase 191 — injected by the daemon binary (which owns
/// capability-checking and the real notification dispatcher)
/// so `aivyx-tool` stays free of any dependency on
/// `aivyx-channel`/`aivyx-capability`. `None` (the default) means
/// this tool process's `DispatchNotification` frames are silently
/// dropped — existing tool processes that never send them are
/// unaffected either way.
///
/// Not `async fn` — implementations that need to await (the real
/// one does, to call `NotifyDispatcher::dispatch`) should spawn
/// their own task internally and return immediately, so the
/// reader loop is never blocked waiting on a notification send.
pub trait NotificationSink: Send + Sync {
    fn dispatch(&self, target: String, message: String, subject: Option<String>);
}
```

- [ ] **Step 2: Add the field to `ToolProcessConfig`, with a manual `Debug` impl**

`ToolProcessConfig` derives `Debug`, but `Arc<dyn NotificationSink>` isn't `Debug` (the trait doesn't require it, and shouldn't — implementors like the real capability-checking sink in Task 3 have no meaningful debug representation). Change:

```rust
/// Configuration for spawning a tool process.
#[derive(Debug, Clone)]
pub struct ToolProcessConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Phase 52 — optional command-wrapper sandbox. When `Some`,
    /// the bridge spawns `wrapper wrapper_args... command
    /// command_args...` instead of `command command_args...`.
    /// Aivyx supplies the policy slot; the operator supplies the
    /// policy (bubblewrap / firejail / Docker / sandbox-exec /
    /// nothing).
    pub sandbox: Option<SandboxConfig>,
}
```

to:

```rust
/// Configuration for spawning a tool process.
#[derive(Clone)]
pub struct ToolProcessConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Phase 52 — optional command-wrapper sandbox. When `Some`,
    /// the bridge spawns `wrapper wrapper_args... command
    /// command_args...` instead of `command command_args...`.
    /// Aivyx supplies the policy slot; the operator supplies the
    /// policy (bubblewrap / firejail / Docker / sandbox-exec /
    /// nothing).
    pub sandbox: Option<SandboxConfig>,
    /// Phase 191 — injected sink for unprompted `DispatchNotification`
    /// frames (see `NotificationSink`). `None` for every tool process
    /// that doesn't need it (the default for all existing callers).
    pub notification_sink: Option<Arc<dyn NotificationSink>>,
}

impl std::fmt::Debug for ToolProcessConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolProcessConfig")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &self.env)
            .field("sandbox", &self.sandbox)
            .field("notification_sink", &self.notification_sink.is_some())
            .finish()
    }
}
```

Every existing call site constructing a `ToolProcessConfig` struct literal (there are several — at minimum the one in this file's own test module, plus every real tool-process spawn site elsewhere in the workspace) will fail to compile without a `notification_sink` field. Grep the whole workspace now: `grep -rn "ToolProcessConfig {" --include="*.rs" .` and add `notification_sink: None,` to every one you find except Task 3's toolkit construction site (which Task 3 itself updates).

- [ ] **Step 3: Thread it through `spawn` into `reader_loop`**

In `ToolProcessBridge::spawn` (line 147), after the existing line `let pending_for_reader = Arc::clone(&pending);` (around line 223) and before the `tokio::spawn` that follows it, add:

```rust
        let notification_sink = config.notification_sink.clone();
```

Change the `tokio::spawn` block from:

```rust
        let reader_handle = tokio::spawn(async move {
            reader_loop(reader, pending_for_reader).await;
        });
```

to:

```rust
        let reader_handle = tokio::spawn(async move {
            reader_loop(reader, pending_for_reader, notification_sink).await;
        });
```

- [ ] **Step 4: Route `DispatchNotification` in `reader_loop`**

Change `reader_loop`'s signature (line 353) from:

```rust
async fn reader_loop(
    mut reader: BufReader<ChildStdout>,
    pending: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<BridgeMessage>>>>,
) {
```

to:

```rust
async fn reader_loop(
    mut reader: BufReader<ChildStdout>,
    pending: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<BridgeMessage>>>>,
    notification_sink: Option<Arc<dyn NotificationSink>>,
) {
```

In its `match msg` block (starts at line 380, currently handling `ToolResult`/`ToolError`/`ToolEvent`/`ToolRegister`), add a new arm:

```rust
            ToolToDaemon::DispatchNotification {
                target,
                message,
                subject,
            } => {
                // No call_id — this doesn't go through `pending` at
                // all, unlike every other variant in this match.
                if let Some(sink) = &notification_sink {
                    sink.dispatch(target, message, subject);
                }
            }
```

Match the existing arms' exact style for how they're written (read the other arms in this same `match` first) — keep this arm's formatting consistent with its neighbors.

- [ ] **Step 5: Write a test proving the no-call_id path works**

The existing test `bridge_handshakes_against_python_inline` (in `bridge.rs`'s `#[cfg(test)] mod tests`) spawns a tiny inline Python tool process, handshakes, and does one `invoke`. Add a new test right after it, following the exact same shape but with the Python script sending an unprompted `DispatchNotification` frame immediately after `ToolRegister` — before ever receiving an `InvokeTool`:

```rust
    #[tokio::test]
    async fn bridge_routes_dispatch_notification_with_no_call_id() {
        let script = r#"
import sys, json, struct

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4:
        return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

hello = read_frame()
assert hello["type"] == "ToolHello"
write_frame({
    "type": "ToolRegister",
    "tool_process_name": "test-tool",
    "tools": []
})

# Unprompted — no InvokeTool preceded this, no call_id at all.
write_frame({
    "type": "DispatchNotification",
    "target": "phone",
    "message": "watcher x went down",
    "subject": None
})
sys.exit(0)
"#;
        let recorded: Arc<Mutex<Vec<(String, String, Option<String>)>>> =
            Arc::new(Mutex::new(Vec::new()));

        struct TestSink(Arc<Mutex<Vec<(String, String, Option<String>)>>>);
        impl NotificationSink for TestSink {
            fn dispatch(&self, target: String, message: String, subject: Option<String>) {
                self.0.blocking_lock().push((target, message, subject));
            }
        }

        let config = ToolProcessConfig {
            name: "test".into(),
            command: "python3".into(),
            args: vec!["-c".into(), script.into()],
            env: vec![],
            sandbox: None,
            notification_sink: Some(Arc::new(TestSink(Arc::clone(&recorded)))),
        };
        let bridge = match ToolProcessBridge::spawn(config).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("skipping: python3 unavailable: {e}");
                return;
            }
        };
        assert_eq!(bridge.descriptors().len(), 0);

        // Give the reader loop a moment to process the frame the
        // Python script sent immediately after ToolRegister.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let calls = recorded.lock().await;
        assert_eq!(calls.len(), 1, "expected exactly one DispatchNotification routed");
        assert_eq!(calls[0].0, "phone");
        assert_eq!(calls[0].1, "watcher x went down");
        assert_eq!(calls[0].2, None);
    }
```

(`Mutex::blocking_lock()` is used inside `TestSink::dispatch` because `NotificationSink::dispatch` is deliberately not `async fn`. Confirmed: `bridge.rs:22` already has `use tokio::sync::{mpsc, Mutex};` in scope — the same `Mutex` type `pending`/`stdin` already use — so `blocking_lock()` is available with no new import.)

- [ ] **Step 6: Run the tests**

Run: `cargo test -p aivyx-tool bridge:: -- --nocapture`
Expected: new test passes; every existing `bridge::` test still passes.

- [ ] **Step 7: Run the full crate suite**

Run: `cargo test -p aivyx-tool`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-tool/src/bridge.rs
git commit -m "feat(tool): route DispatchNotification through an injectable sink — Phase 191"
```

---

### Task 3: Capability scope + daemon-side sink implementation and wiring

**Files:**
- Modify: `crates/aivyx-capability/src/lib.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (or `crates/aivyx-channel/src/notify_dispatcher.rs` — see Step 3 for how to decide which)
- Test: wherever the chosen file's existing tests live

**Interfaces:**
- Consumes: `NotificationSink` trait from Task 2; `NotifyDispatcher::dispatch(&self, target_name: &str, message: &str, subject: Option<&str>) -> Result<(), NotifyError>` (already exists, `crates/aivyx-channel/src/notify_dispatcher.rs:183-196`); `CapabilitySet::grants(&self, needed: &Scope) -> bool` (already exists, `crates/aivyx-capability/src/lib.rs:868`); the daemon's already-computed `capabilities: CapabilitySet` (`crates/aivyx-cli/src/bin/aivyx.rs:8204`, the role∩tier intersection every other capability check in the daemon already uses).
- Produces: a `NotifyDispatcherSink` (or similar name) type implementing `NotificationSink`, constructed with a clone of `capabilities`, a clone of `Arc<NotifyDispatcher>`, and the configured target name (from Task 4's config field — for this task, accept the target name as a plain `String` parameter; Task 4 wires the real config value in).

- [ ] **Step 1: Add the capability scope**

In `crates/aivyx-capability/src/lib.rs`, find the `KNOWN_BASES` list (search for that exact identifier — it's described in this repo's own `CLAUDE.md` as "~90 scope strings"). Add `"notify.dispatch"` to it, following the exact same style as neighboring entries (alphabetical or grouped by feature — match whatever ordering convention the surrounding entries already use; don't impose a new one).

- [ ] **Step 2: Write a parse test**

Find `KNOWN_BASES`' existing test coverage (search for a test that asserts a known scope parses, e.g. something exercising `Scope::parse` against an existing base like `"health.write"` or similar) and add an equivalent for the new one:

```rust
    #[test]
    fn notify_dispatch_scope_parses() {
        assert!(Scope::parse("notify.dispatch").is_some());
    }
```

(Adjust to match whatever the neighboring test's exact assertion style is — some of this crate's tests may use `.is_ok()` instead of `.is_some()` depending on `Scope::parse`'s real return type; confirm from the function signature before writing the assertion.)

- [ ] **Step 3: Decide where the sink type lives**

`NotifyDispatcher` is defined in `aivyx-channel`; `NotificationSink` is defined in `aivyx-tool`; `aivyx-cli` depends on both. Rust's orphan rule permits implementing a foreign trait (`NotificationSink`, from `aivyx-tool`) for a foreign type (`NotifyDispatcher`, from `aivyx-channel`) only from a crate that owns *one* of them — neither `aivyx-channel` nor `aivyx-tool` owns the other, and `aivyx-cli` owns neither, so a direct `impl NotificationSink for NotifyDispatcher` cannot live in `aivyx-cli`. Two real options; pick whichever compiles cleanly and report which in your task report:
   - **Option A** (likely simplest): define a new local wrapper struct in `aivyx-cli/src/bin/aivyx.rs` itself, e.g. `struct ToolkitNotifySink { dispatcher: Arc<NotifyDispatcher>, capabilities: CapabilitySet, target: String }`, and `impl NotificationSink for ToolkitNotifySink` there — `aivyx-cli` owns `ToolkitNotifySink` (a local type), so implementing the foreign `NotificationSink` trait for it is allowed under the orphan rule regardless of where `NotifyDispatcher` lives.
   - **Option B**: add `aivyx-tool` as a dependency of `aivyx-channel` (reasonable direction — `aivyx-channel` already depends on many substrate crates; the *forbidden* direction per Global Constraints is `aivyx-tool` depending on `aivyx-channel`, not the reverse) and implement `NotificationSink` for `NotifyDispatcher` directly inside `aivyx-channel`, since `aivyx-channel` owns `NotifyDispatcher`.

   Prefer Option A unless it hits a real blocker — it's a smaller diff and doesn't touch `aivyx-channel`'s `Cargo.toml` at all.

- [ ] **Step 4: A real sequencing problem — neither `capabilities` nor `notify_dispatcher` exists yet where `ToolProcessConfig` is built**

Confirmed against the real file, two separate ordering facts: the tool-process spawn loop that constructs every `ToolProcessConfig` (`crates/aivyx-cli/src/bin/aivyx.rs:7763-7769`, `let spawn_cfg = aivyx_tool::ToolProcessConfig { name: tp_cfg.name.clone(), ... };`) is a **generic loop over every operator-configured `[[tool_process]]` entry** — not toolkit-specific — and it runs to completion (line ~7841) entirely *before either*:
- `notify_dispatcher` is built (`let notify_dispatcher = aivyx_channel::notify_dispatcher::build_notify_dispatcher(...)`, line 7967), or
- `capabilities` is computed (`let capabilities = role_envelope.intersect(role_tier_ceiling);`, line 8204).

This loop also doesn't check anything against a role's granted capability set today — it only resolves each tool's *declared* scope string (optionally narrowed by `tp_cfg.scope_overrides`) and wraps it in a `ToolProxy`; the real grant check happens later, generically, whenever a tool actually gets invoked. There is no existing per-tool-process capability or dispatcher value to reuse at line 7763 — naive references to either `capabilities` or `notify_dispatcher` there would not compile.

Fix: defer both values together in a single fill-once cell, set later once both are available (by line 8204, the later of the two). Add near the top of `aivyx.rs` (or immediately before the spawn loop at ~7763):

```rust
type NotifyDispatchDeps = (
    aivyx_capability::CapabilitySet,
    std::sync::Arc<aivyx_channel::notify_dispatcher::NotifyDispatcher>,
);

struct ToolkitNotifySink {
    // Filled in once, right after both `notify_dispatcher` (line
    // 7967) and `capabilities` (line 8204) exist — the spawn loop
    // that constructs this sink (line 7763) runs before either
    // value does. `dispatch()` sees `None` only if a frame arrives
    // during daemon startup itself, before any tool process could
    // plausibly have connected — treat that as ungranted (deny,
    // don't panic, don't guess).
    deps: std::sync::Arc<std::sync::OnceLock<NotifyDispatchDeps>>,
    target: String,
}

/// Pure, directly-testable: does this capability set grant the
/// push-notification scope? Kept separate from
/// `NotificationSink::dispatch` specifically so it doesn't need an
/// async mock or a spawned task to test (Step 6).
fn notify_dispatch_granted(capabilities: &aivyx_capability::CapabilitySet) -> bool {
    let needed = aivyx_capability::Scope::parse("notify.dispatch")
        .expect("notify.dispatch must parse — added to KNOWN_BASES in Task 3 Step 1");
    capabilities.grants(&needed)
}

impl aivyx_tool::bridge::NotificationSink for ToolkitNotifySink {
    fn dispatch(&self, target: String, message: String, subject: Option<String>) {
        let Some((capabilities, dispatcher)) = self.deps.get() else {
            // Startup not finished yet — deny, don't guess.
            eprintln!(
                "aivyx: tool process denied notify.dispatch (target {target}); \
                 daemon startup not complete"
            );
            return;
        };
        if !notify_dispatch_granted(capabilities) {
            eprintln!(
                "aivyx: tool process denied notify.dispatch (target {target}); \
                 capability not held by the active role"
            );
            return;
        }
        // The wire frame's own `target` is currently unused in favor of
        // the configured default — Task 4 wires the real
        // `default_notify_target` value into `self.target`; both fields
        // exist on the frame (mirroring `notify.send`'s own shape) but
        // this phase only supports one target per toolkit process, so
        // `self.target` (the configured default) wins. If they ever
        // diverge, that's a signal per-watcher targets (explicitly out
        // of scope) are wanted — not a bug to silently paper over.
        let dispatcher = std::sync::Arc::clone(dispatcher);
        let target_name = self.target.clone();
        tokio::spawn(async move {
            if let Err(e) = dispatcher
                .dispatch(&target_name, &message, subject.as_deref())
                .await
            {
                eprintln!("aivyx: notify.dispatch failed (target {target_name}): {e}");
            }
        });
        let _ = target; // see comment above — frame's own target intentionally unused for now
    }
}
```

- [ ] **Step 5: Wire the cell at both ends**

Just before the spawn loop (~line 7763), construct the empty cell:

```rust
    let notify_deps: std::sync::Arc<std::sync::OnceLock<NotifyDispatchDeps>> =
        std::sync::Arc::new(std::sync::OnceLock::new());
```

Inside the loop, when building `spawn_cfg` (line 7763-7769), add the new field — Task 4 replaces the placeholder empty `target` with the real configured value once that config is loaded, and this sink applies to *every* spawned tool process generically (matching the design spec's "general-purpose by construction" framing — the capability check, not a process-name check, is what actually gates behavior):

```rust
        let spawn_cfg = aivyx_tool::ToolProcessConfig {
            name: tp_cfg.name.clone(),
            command: tp_cfg.command.clone(),
            args: tp_cfg.args.clone(),
            env: tp_cfg.env.clone(),
            sandbox: spawn_sandbox,
            notification_sink: Some(std::sync::Arc::new(ToolkitNotifySink {
                deps: std::sync::Arc::clone(&notify_deps),
                target: String::new(), // Task 4 replaces this with the real configured value
            }) as std::sync::Arc<dyn aivyx_tool::bridge::NotificationSink>),
        };
```

Immediately after `capabilities` is computed (line 8204 — the later of the two dependencies, since `notify_dispatcher` already exists by then from line 7967), fill the cell:

```rust
    let _ = notify_deps.set((capabilities.clone(), std::sync::Arc::clone(&notify_dispatcher)));
```

(`OnceLock::set` returns `Err` if already set — impossible here since this is the only call site, but the `let _ =` deliberately doesn't unwrap/panic on it, since a startup-path panic over a notify-dispatch wiring detail would be a worse failure mode than silently keeping the first-set value.)

- [ ] **Step 6: Write a unit test for the capability gate**

Test the extracted pure function directly — no async mocking, no spawned-task inspection needed:

```rust
    #[test]
    fn notify_dispatch_granted_true_when_scope_held() {
        let caps = aivyx_capability::CapabilitySet::from_scopes(vec![
            aivyx_capability::Scope::parse("notify.dispatch").unwrap(),
        ]);
        assert!(notify_dispatch_granted(&caps));
    }

    #[test]
    fn notify_dispatch_granted_false_when_scope_absent() {
        let caps = aivyx_capability::CapabilitySet::from_scopes(vec![
            aivyx_capability::Scope::parse("health.write").unwrap(),
        ]);
        assert!(!notify_dispatch_granted(&caps));
    }

    #[test]
    fn notify_dispatch_granted_false_for_empty_capability_set() {
        assert!(!notify_dispatch_granted(&aivyx_capability::CapabilitySet::empty()));
    }
```

Add these to `aivyx.rs`'s existing `#[cfg(test)]` module (search for it — this is a large binary crate file, confirm the module exists before assuming where to add new tests; if genuinely none exists, add a new `#[cfg(test)] mod notify_dispatch_sink_tests { use super::*; ... }` block near `ToolkitNotifySink`'s definition).

- [ ] **Step 7: Run the tests**

Run: `cargo test -p aivyx-capability notify_dispatch` and `cargo test -p aivyx-cli notify_dispatch` (the second may take longer to compile — this is a large binary crate; let it run to completion).
Expected: all pass, including the new tests from Steps 2 and 6.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-capability/src/lib.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat: capability-gated notify-dispatch sink for tool processes — Phase 191"
```

---

### Task 4: Toolkit stdout-sharing + `default_notify_target` config

**Files:**
- Modify: `crates/aivyx-tool/src/multi_harness.rs`
- Modify: `crates/aivyx-toolkit/src/config.rs`
- Modify: `crates/aivyx-toolkit/src/main.rs`

**Interfaces:**
- Consumes: `ToolToDaemon::DispatchNotification` (Task 1).
- Produces: `run_multi_tool_subprocess`'s new optional parameter (exact name `extra_outbound: Option<tokio::sync::mpsc::UnboundedReceiver<ToolToDaemon>>`); `ToolkitConfig`'s new `default_notify_target: Option<String>` field. Task 5 (health_polling.rs) is the actual sender on the `mpsc::UnboundedSender` half of this channel.

- [ ] **Step 1: Read `run_multi_tool_subprocess`'s current body**

Read the whole function in `crates/aivyx-tool/src/multi_harness.rs` (starts at line 117, already partially shown above: builds `by_name`, then `let mut stdin = stdin(); let stdout_sink = Arc::new(Mutex::new(stdout()));`). Confirm the exact rest of the function — how it spawns its own reader/writer tasks — before editing, since this plan cannot show the full pre-edit body.

- [ ] **Step 2: Add the optional extra-outbound-frames parameter**

Change the signature to:

```rust
pub async fn run_multi_tool_subprocess(
    tools: Vec<Arc<dyn Tool>>,
    tool_process_name: impl Into<String>,
    extra_outbound: Option<tokio::sync::mpsc::UnboundedReceiver<crate::wire::ToolToDaemon>>,
) -> Result<(), HarnessError> {
```

(Existing callers elsewhere in the workspace that invoke `run_multi_tool_subprocess` with only 2 arguments will fail to compile. This is a genuinely large blast radius — confirmed via `grep -rln "run_multi_tool_subprocess" crates/*/src --include="*.rs"`: 23 files across `aivyx-apps`, `aivyx-calendar`, `aivyx-contacts`, `aivyx-drive`, `aivyx-gmail`, `aivyx-n8n`, `aivyx-notion`, `aivyx-obsidian`, `aivyx-toolkit`, `aivyx-vertical-sdk`, plus `aivyx-tool` itself — most tool-process crates have both a real `.await` call site (typically in `main.rs`) and doc/comment mentions in `lib.rs`/`harness.rs` that don't need editing. Run that grep now, open each hit, and add `None` as the third argument to every *real call expression* (not doc comments) except the toolkit's own call site from Task 6 Step 4, which passes `Some(...)`. `cargo build --workspace` (Step 8, below) is the actual completeness check — don't trust the grep alone to have found every real call site, some may be behind a macro or a re-exported wrapper function.)

- [ ] **Step 3: Forward frames from the channel through the shared `stdout_sink`**

After the existing `let stdout_sink = Arc::new(Mutex::new(stdout()));` line, add a small forwarding task that only runs if `extra_outbound` is `Some`:

```rust
    if let Some(mut rx) = extra_outbound {
        let forward_sink = Arc::clone(&stdout_sink);
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                let mut guard = forward_sink.lock().await;
                let _ = crate::frame::write_frame(&mut *guard, &frame).await;
            }
        });
    }
```

Place this after `stdout_sink` is constructed but before the function's own reader loop starts consuming `stdin` (so both writers are live for the whole run). Confirm `crate::frame::write_frame`'s exact signature matches this call (it's the same function `harness.rs` already uses — `write_frame<W: AsyncWrite + Unpin, T: Serialize>`) — the shared `Mutex<Stdout>` guard `&mut *guard` should satisfy `AsyncWrite + Unpin` the same way the existing harness code already relies on.

- [ ] **Step 4: Add the config field**

In `crates/aivyx-toolkit/src/config.rs`, find `ToolkitConfig`'s struct definition and its `[toolkit]`-section parsing (search for how the existing config fields are declared and parsed — follow that exact pattern, likely a `serde(default)` optional field on a TOML-deserialized struct). Add:

```rust
    /// Phase 191 — the notify target health-check alerts dispatch
    /// to automatically. `None` (unset) means alerts are skipped
    /// silently; see `health_polling.rs`.
    #[serde(default)]
    pub default_notify_target: Option<String>,
```

- [ ] **Step 5: Write a config-parsing test**

Find `config.rs`'s existing test module and add a test confirming a `[toolkit]` block with `default_notify_target = "phone"` parses to `Some("phone".to_string())`, and a block omitting it parses to `None` — following the exact same test pattern the file's existing field tests already use (e.g. however `brave_search.api_key` or similar is tested today).

- [ ] **Step 6: Wire the channel in `main.rs`**

In `crates/aivyx-toolkit/src/main.rs`, before the existing `tokio::spawn(async move { run_polling_loop(polling_store, polling_http).await })` (around line 106) and before the `run_multi_tool_subprocess(tools, "aivyx-toolkit")` call (around line 157), construct the channel:

```rust
    let (notify_tx, notify_rx) = tokio::sync::mpsc::unbounded_channel();
```

Pass `notify_tx` into the polling loop's spawn (Task 5 changes `run_polling_loop`'s signature to accept it — for this task, just thread the value through; the parameter name/type is fixed by Task 5's interface below) and pass `Some(notify_rx)` as `run_multi_tool_subprocess`'s new third argument.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p aivyx-tool multi_harness::` and `cargo test -p aivyx-toolkit config::`
Expected: all pass, including new tests from Steps 3 (implicitly, via existing harness tests still passing with the new optional param defaulted to `None`) and 5.

- [ ] **Step 8: Full-workspace compile check**

Run: `cargo build --workspace` (or at minimum every crate that calls `run_multi_tool_subprocess`, per your Step 2 grep) to confirm every call site compiles with the new parameter.
Expected: exits 0.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-tool/src/multi_harness.rs crates/aivyx-toolkit/src/config.rs crates/aivyx-toolkit/src/main.rs
git commit -m "feat(toolkit): stdout-sharing channel + default_notify_target config — Phase 191"
```

---

### Task 5: `health_store.rs` — surface detected transitions

**Files:**
- Modify: `crates/aivyx-toolkit/src/health_store.rs`

**Interfaces:**
- Produces: `record_check`'s new return type, `Result<Option<Transition>, HealthStoreError>` — `Some(transition)` iff a real state flip was detected this call, `None` otherwise (including the "first-ever poll" and "no change" cases, which return `Ok(None)`, not an error).

- [ ] **Step 1: Write the failing test**

In `health_store.rs`'s existing `#[cfg(test)] mod tests` block, add (following the exact fixture/setup pattern the neighboring tests like `record_check_state_flip_records_transition` already use — read that test first and mirror its setup):

```rust
    #[tokio::test]
    async fn record_check_returns_the_transition_on_a_real_flip() {
        let dir = scratch_dir();
        let store = HealthStore::open(dir.join("health.json")).await.unwrap();
        store.add_watcher("x".to_string(), "https://x/".to_string(), 60, 200).await.unwrap();
        // First poll: establishes a baseline, no transition.
        let first = store
            .record_check("x", ProbeOutcome { status_code: Some(200), ok: true }, t0())
            .await
            .unwrap();
        assert!(first.is_none(), "first-ever poll must not report a transition");

        // Second poll: real flip.
        let second = store
            .record_check(
                "x",
                ProbeOutcome { status_code: Some(503), ok: false },
                t0() + chrono::Duration::seconds(60),
            )
            .await
            .unwrap();
        assert!(second.is_some(), "an ok-flag flip must report the transition");
        let t = second.unwrap();
        assert_eq!(t.watcher_name, "x");
        assert!(t.from_ok);
        assert!(!t.to_ok);
        assert_eq!(t.status_code, Some(503));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn record_check_returns_none_when_state_unchanged() {
        let dir = scratch_dir();
        let store = HealthStore::open(dir.join("health.json")).await.unwrap();
        store.add_watcher("x".to_string(), "https://x/".to_string(), 60, 200).await.unwrap();
        store.record_check("x", ProbeOutcome { status_code: Some(200), ok: true }, t0()).await.unwrap();
        let repeat = store
            .record_check(
                "x",
                ProbeOutcome { status_code: Some(200), ok: true },
                t0() + chrono::Duration::seconds(60),
            )
            .await
            .unwrap();
        assert!(repeat.is_none(), "no state change must not report a transition");
        std::fs::remove_dir_all(&dir).ok();
    }
```

This mirrors the exact fixture pattern the file's own pre-existing `record_check_first_poll_no_transition`, `record_check_state_flip_records_transition`, and `record_check_no_state_change_no_transition` tests already use (`scratch_dir()`, `HealthStore::open`, `add_watcher("x".to_string(), "https://x/".to_string(), 60, 200)`, the `t0()` helper at line 558) — those three tests already assert against `recent_transitions_within(...)` to prove the *ring buffer* got the right entry; these two new tests assert against `record_check`'s own *return value* instead, since that's the new signal this task adds. Both checks are complementary, not redundant — keep the existing three tests unchanged.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-toolkit record_check_returns_the_transition_on_a_real_flip -- --nocapture`
Expected: FAIL — compile error, since `record_check` doesn't return `Option<Transition>` yet.

- [ ] **Step 3: Change `record_check`'s return type**

In `crates/aivyx-toolkit/src/health_store.rs`, change:

```rust
    pub async fn record_check(
        &self,
        watcher_name: &str,
        outcome: ProbeOutcome,
        now: DateTime<Utc>,
    ) -> Result<(), HealthStoreError> {
```

to:

```rust
    pub async fn record_check(
        &self,
        watcher_name: &str,
        outcome: ProbeOutcome,
        now: DateTime<Utc>,
    ) -> Result<Option<Transition>, HealthStoreError> {
```

Then change the body's transition-push block from:

```rust
        if had_prior_check {
            if let Some(prev) = prev_ok {
                if prev != outcome.ok {
                    push_transition_ring(
                        &mut guard.recent_transitions,
                        Transition {
                            watcher_name: watcher_name.to_string(),
                            transitioned_at: now,
                            from_ok: prev,
                            to_ok: outcome.ok,
                            status_code: outcome.status_code,
                        },
                    );
                }
            }
        }

        save_to_disk(&self.path, &guard).await?;
        Ok(())
    }
```

to:

```rust
        let mut fired: Option<Transition> = None;
        if had_prior_check {
            if let Some(prev) = prev_ok {
                if prev != outcome.ok {
                    let t = Transition {
                        watcher_name: watcher_name.to_string(),
                        transitioned_at: now,
                        from_ok: prev,
                        to_ok: outcome.ok,
                        status_code: outcome.status_code,
                    };
                    push_transition_ring(&mut guard.recent_transitions, t.clone());
                    fired = Some(t);
                }
            }
        }

        save_to_disk(&self.path, &guard).await?;
        Ok(fired)
    }
```

(`Transition` needs `Clone` for this — check its `#[derive(...)]` at its struct definition, ~line 142; add `Clone` if it isn't already derived.)

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p aivyx-toolkit record_check_returns -- --nocapture`
Expected: both new tests pass.

- [ ] **Step 5: Fix the existing caller**

`record_check`'s only caller is in `health_polling.rs` — grep for it (`grep -n "record_check" crates/aivyx-toolkit/src/health_polling.rs`) and update it to handle the new return type. At this point (before Task 6), a minimal fix is enough to keep the crate compiling: discard the returned `Option<Transition>` explicitly (Task 6 replaces this with real handling):

```rust
    let _ = store.record_check(&watcher.name, outcome, now).await;
```

(Match whatever the real existing call site looks like — this may need `?` error propagation preserved from the original, only the success-value handling changes.)

- [ ] **Step 6: Run the full crate suite**

Run: `cargo test -p aivyx-toolkit`
Expected: all pass, including the pre-existing `record_check_state_flip_records_transition` and `record_check_no_state_change_no_transition` tests (which still exercise the same logic, just via the new return type — read them and confirm they don't need updating, or update them minimally if they assert on the old `Result<(), _>` shape).

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-toolkit/src/health_store.rs crates/aivyx-toolkit/src/health_polling.rs
git commit -m "feat(toolkit): record_check surfaces detected transitions — Phase 191"
```

---

### Task 6: `health_polling.rs` — dispatch on transition

**Files:**
- Modify: `crates/aivyx-toolkit/src/health_polling.rs`
- Modify: `crates/aivyx-toolkit/src/main.rs`

**Interfaces:**
- Consumes: `record_check`'s `Option<Transition>` (Task 5); `run_polling_loop`'s new sender parameter and the `mpsc::UnboundedSender<ToolToDaemon>` half constructed in Task 4 Step 6; `ToolkitConfig::default_notify_target` (Task 4).
- Produces: nothing later tasks depend on — this is the last functional task; Task 7 is docs only.

- [ ] **Step 1: Thread the sender and target into `run_polling_loop`**

Change `run_polling_loop`'s signature from:

```rust
pub async fn run_polling_loop(store: Arc<HealthStore>, http: Client) -> ! {
```

to:

```rust
pub async fn run_polling_loop(
    store: Arc<HealthStore>,
    http: Client,
    notify_tx: tokio::sync::mpsc::UnboundedSender<aivyx_tool::wire::ToolToDaemon>,
    default_notify_target: Option<String>,
) -> ! {
```

Confirmed the exact current body of `run_polling_tick` (`crates/aivyx-toolkit/src/health_polling.rs:65-81`):

```rust
pub async fn run_polling_tick(
    store: &HealthStore,
    http: &Client,
    now: chrono::DateTime<Utc>,
) {
    let due = store.due_watchers(now).await;
    for watcher in due {
        let outcome = probe(http, &watcher).await;
        let _ = store.record_check(&watcher.name, outcome, Utc::now()).await;
    }
}
```

(Note the existing quirk, unrelated to this task and not to be "fixed" here: the record itself uses a fresh `Utc::now()`, not the `now` parameter — `now` is only used for `due_watchers(now)`. Preserve this exactly; changing it is out of scope.)

Change its signature to add the two new parameters, matching `run_polling_loop`'s Step 1 addition:

```rust
pub async fn run_polling_tick(
    store: &HealthStore,
    http: &Client,
    now: chrono::DateTime<Utc>,
    notify_tx: &tokio::sync::mpsc::UnboundedSender<aivyx_tool::wire::ToolToDaemon>,
    default_notify_target: &Option<String>,
) {
```

(By reference, not by value — this function is called once per due watcher inside `run_polling_loop`'s own loop, so it shouldn't consume/move either.)

Update `run_polling_loop`'s call site to pass them through:

```rust
        run_polling_tick(&store, &http, Utc::now(), &notify_tx, &default_notify_target).await;
```

- [ ] **Step 2: Compose and send the notification on a real transition**

Change the loop body from:

```rust
    for watcher in due {
        let outcome = probe(http, &watcher).await;
        let _ = store.record_check(&watcher.name, outcome, Utc::now()).await;
    }
```

to:

```rust
    for watcher in due {
        let outcome = probe(http, &watcher).await;
        if let Ok(Some(transition)) = store.record_check(&watcher.name, outcome, Utc::now()).await {
            if let Some(target) = default_notify_target {
                let direction = if transition.to_ok { "RECOVERED" } else { "DOWN" };
                let status = transition
                    .status_code
                    .map(|c| format!(" (status {c})"))
                    .unwrap_or_default();
                let message = format!(
                    "Health watcher '{}' went {direction}{status}",
                    transition.watcher_name
                );
                let _ = notify_tx.send(aivyx_tool::wire::ToolToDaemon::DispatchNotification {
                    target: target.clone(),
                    message,
                    subject: Some("Health alert".to_string()),
                });
            }
            // default_notify_target unset: skip silently, per Global
            // Constraints — no notify_tx.send attempted at all.
        }
    }
```

- [ ] **Step 3: Write tests for both directions**

Add tests to `health_polling.rs`'s existing test module (`#[cfg(test)] mod tests`, starting line 113), following the exact fixture pattern its own `polling_tick_updates_store_with_probe_outcome` test already uses (`scratch_dir()`, `spawn_mock_server(status)`, `HealthStore::open`, `store.add_watcher(...)`):

```rust
    #[tokio::test]
    async fn tick_sends_dispatch_notification_on_down_transition() {
        let dir = scratch_dir();
        let path = dir.join("health.json");
        let store = Arc::new(HealthStore::open(path).await.unwrap());
        let ok_url = spawn_mock_server(200).await;
        store.add_watcher("x".to_string(), ok_url, 60, 200).await.unwrap();
        let http = Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // First tick: establishes the "up" baseline, no transition yet.
        run_polling_tick(&store, &http, Utc::now(), &tx, &Some("phone".to_string())).await;
        assert!(rx.try_recv().is_err(), "no transition on the first-ever poll");

        // Rewrite the watcher's URL to a failing mock (same technique the
        // existing polling_tick_records_transition_on_state_flip test
        // already uses — no direct "update watcher URL" store API exists).
        let down_url = spawn_mock_server(503).await;
        let watchers_path = dir.join("health.json");
        let body = std::fs::read_to_string(&watchers_path).unwrap();
        let body = body.replace(&format!("\"url\": \"{ok_url}\""), &format!("\"url\": \"{down_url}\""));
        std::fs::write(&watchers_path, body).unwrap();
        let store = Arc::new(HealthStore::open(watchers_path).await.unwrap());

        run_polling_tick(&store, &http, Utc::now(), &tx, &Some("phone".to_string())).await;
        let sent = rx.try_recv().expect("expected a DispatchNotification on the down transition");
        match sent {
            aivyx_tool::wire::ToolToDaemon::DispatchNotification { target, message, .. } => {
                assert_eq!(target, "phone");
                assert!(message.contains("DOWN"), "message was: {message}");
            }
            other => panic!("expected DispatchNotification, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn tick_sends_nothing_when_no_target_configured() {
        let dir = scratch_dir();
        let path = dir.join("health.json");
        let store = Arc::new(HealthStore::open(path).await.unwrap());
        let url = spawn_mock_server(503).await; // fails immediately, no baseline needed
        store.add_watcher("x".to_string(), url, 60, 200).await.unwrap();
        let http = Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        run_polling_tick(&store, &http, Utc::now(), &tx, &None).await;
        assert!(rx.try_recv().is_err(), "no target configured means no send at all, even on a real transition");
        std::fs::remove_dir_all(&dir).ok();
    }
```

(`ok_url`/`down_url` string-replace against the raw JSON file mirrors exactly what the existing `polling_tick_records_transition_on_state_flip` test in this same file already does at line ~266-278 to change a watcher's URL between ticks — there's no watcher-update API to call directly.)

- [ ] **Step 4: Update `main.rs`'s call site**

In `crates/aivyx-toolkit/src/main.rs`, update the `tokio::spawn(async move { run_polling_loop(polling_store, polling_http).await })` call (from Task 4 Step 6, which already added `notify_tx`) to also pass the configured target:

```rust
    tokio::spawn(async move {
        run_polling_loop(polling_store, polling_http, notify_tx, config.default_notify_target.clone()).await
    });
```

(`config` here is whatever the existing loaded `ToolkitConfig` local binding is named — confirm from context; it must already be in scope before this spawn, since Task 4 added the field to the same struct this binding has.)

- [ ] **Step 5: Run the tests**

Run: `cargo test -p aivyx-toolkit health_polling::`
Expected: all pass, including the two new tests.

- [ ] **Step 6: Run the full crate + workspace suite**

Run: `cargo test -p aivyx-toolkit && cargo build --workspace`
Expected: all pass, full workspace compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-toolkit/src/health_polling.rs crates/aivyx-toolkit/src/main.rs
git commit -m "feat(toolkit): dispatch automatic health-check alerts on transitions — Phase 191"
```

---

### Task 7: Documentation

**Files:**
- Modify: `docs/INSTALL.md`
- Modify: `docs/TOOL_SDK.md`

**Interfaces:**
- Consumes: nothing (docs-only task, can run any time after Task 6 lands, since it describes the finished feature).

- [ ] **Step 1: Correct `docs/INSTALL.md`'s stale "What Phase 125 deliberately leaves to follow-on phases" section**

Find the section (search for `"What Phase 125 deliberately leaves to follow-on phases"`). Remove the bullet:

```
- **No `health.check.remove` tool.** Operators can manually
  edit `health.json` until a proper remove tool ships.
```

(stale — shipped Phase 147; `health.check.remove` already exists per `crates/aivyx-toolkit/src/tools/health_check.rs`).

Replace the bullet:

```
- **No automatic alert dispatch.** The agent composes
  alerts from `health.check.recent_changes`; the daemon
  doesn't auto-call `notify.send`. Phase 126+ may add a
  daemon-side IPC hook for tool processes to dispatch
  notifications directly.
```

with:

```
- ~~No automatic alert dispatch~~ — **shipped Phase 191.** Set
  `[toolkit] default_notify_target` in
  `~/.aivyx/tool-processes/toolkit/config.toml` and the polling
  loop dispatches a notification directly (both directions: down
  and recovered) whenever `health.check.recent_changes` would
  have shown a new entry — no cron, no agent turn required. The
  agent-mediated recipe below still works and is useful for
  richer, LLM-composed alert text; the automatic path is a
  reliable floor under it, not a replacement.
```

- [ ] **Step 2: Update the "Health-monitoring alert composition recipe" section**

Immediately above that section's existing prose (`"The polling loop records state transitions; the **agent composes alerts**. Substrate-minimal per Phase 125 — no daemon-side automatic alert dispatch (deferred to Phase 126+)."`), replace with:

```
The polling loop records state transitions and, since Phase 191, also
dispatches a plain automatic notification for every transition directly
(see `[toolkit] default_notify_target` above) — no cron or agent turn
required for that baseline case. The recipe below remains useful when you
want the *agent* to compose richer, more specific alert text instead of
(or alongside) the automatic one.
```

- [ ] **Step 3: Update `docs/TOOL_SDK.md`**

`crates/aivyx-tool/src/wire.rs`'s own doc comment states it is "the authoritative source for the schema documented in `docs/TOOL_SDK.md`." Find `docs/TOOL_SDK.md`'s existing documentation of the `ToolToDaemon` wire variants (search for `ToolRegister` or `ToolResult` to find the right section) and add a new entry for `DispatchNotification`, following the exact same format the existing variants are documented in (field list + one-paragraph description). Describe: no `call_id` (unprompted, not a response to any `InvokeTool`); requires the tool process to hold the `notify.dispatch` capability scope (checked by the daemon, not by `aivyx-tool` itself); silently dropped if the tool process isn't granted that scope.

- [ ] **Step 4: Commit**

```bash
git add docs/INSTALL.md docs/TOOL_SDK.md
git commit -m "docs: document automatic alert dispatch (Phase 191) + correct stale Phase 125 follow-ups"
```

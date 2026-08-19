# Wire the remaining `aivyx-checkpoint` construction sites Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thread the single `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>` instance `aivyx.rs` already builds once (near `canonical_root`) into the 5 real `ConcreteAgent` construction sites the prior `aivyx-checkpoint` adoption project left unwired: `aivyx-discord`, `aivyx-slack`, `aivyx-telegram`'s session chains, and `aivyx-team`'s two agent-construction paths (the daemon's team-mission driver and the CLI's own `team run` command).

**Architecture:** Pure parameter-threading, mirroring the adoption project's own Task 4 exactly. No new `GitCheckpointer` is ever constructed in this plan — every site receives a clone of the one instance `aivyx.rs`'s `run()` already builds. Each channel crate (`aivyx-discord`/`aivyx-slack`/`aivyx-telegram`) has an identical 3-hop call chain (`pub run_X_session` → `pub(crate) run_X_session_with_transport` → `pub(crate) run_X_session_with_mailbox`, spawned per-channel via `tokio::spawn`) where the real `ConcreteAgent::new(...)` lives at the innermost hop. `aivyx-team`'s path is deeper: `SpecialistFactory` (new field + builder) → `TeamAssembly::build` (new parameter) → two real callers, `aivyx-channel`'s `team_mission_driver.rs` (via a new `TeamRunDeps` field) and `aivyx-cli`'s own `team.rs::run_mission` (which also builds its own lead `ConcreteAgent` directly).

**Tech Stack:** Rust, edition 2024. No new dependencies anywhere in this plan.

## Global Constraints

- Design doc: `docs/superpowers/specs/2026-08-19-aivyx-checkpoint-remaining-sites-design.md`.
- Every site receives `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>` (or a `.clone()` of it) — never constructs a new `GitCheckpointer`.
- `run_telegram_session` (the singular, non-multi variant) has zero real callers anywhere in the workspace — confirmed via a full-workspace grep during design. **Do not touch it.** Only `run_telegram_multi_session` and its chain are in scope.
- `aivyx-channel::session::build_agent_stack`'s "other caller" does not exist — the adoption project's own prior Task 4 already covers every real call site of that function. **Nothing in this plan touches `aivyx-channel/src/session.rs`.**
- `git.rs`'s three tools and `workspace.*` tools remain out of scope — unchanged from the original adoption.
- No new operator-facing config anywhere in this plan.

---

## Task 1: `aivyx-discord`

**Files:**
- Modify: `crates/aivyx-discord/src/session.rs` (3 function signatures + 1 spawn-site clone + 1 builder-chain addition)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the one real `run_discord_session(...)` call site)
- Test: `crates/aivyx-discord/src/tests.rs`

**Interfaces:**
- Consumes: `aivyx_core::GitCheckpointer` (already re-exported from `aivyx-core`), the `checkpointer` binding already built in `aivyx.rs` near `canonical_root`.
- Produces: `run_discord_session`, `run_discord_session_with_transport`, `run_discord_session_with_mailbox` all gain a new `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>` parameter, in that order down the chain.

- [ ] **Step 1: Write the failing test**

In `crates/aivyx-discord/src/tests.rs`, add (near the existing `discord_session_smoke_e2e` test — reuses its `scratch_storage`/`discord_session_config` helpers, but needs its own `fs_root` since it exercises real `fs.write`):

```rust
// ---------------------------------------------------------------------------
// Test — a real dispatched fs.write through the real discord construction
// chain produces a real, restorable git checkpoint.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discord_dispatched_fs_write_produces_a_checkpoint() {
    let (storage, parent) = scratch_storage("checkpoint").await;

    // A real git-backed fs_root, separate from the audit/memory scratch dir.
    let fs_root = std::env::temp_dir().join(format!(
        "aivyx-discord-checkpoint-fsroot-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&fs_root).unwrap();
    aivyx_checkpoint::test_support::init_repo(&fs_root).await;

    let write_tool: Arc<dyn Tool> = Arc::new(
        aivyx_core::tools::fs::FsWriteToolConfig::new(fs_root.clone())
            .build()
            .expect("fs_root must be canonicalizable"),
    );

    // One ToolCalls step (fs.write) followed by one FinalMessage step
    // closing the turn — same shape aivyx-telegram's own
    // run_telegram_session_two_chats_persistent_e2e test uses for its
    // memory.write script, the closest prior art in this codebase.
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(
            vec![
                ScriptedStep {
                    events: vec![],
                    terminal: LlmStepEnd::ToolCalls {
                        calls: vec![aivyx_llm::ToolCallEnd {
                            call_id: "toolu_1".to_string(),
                            tool_name: "fs.write".to_string(),
                            input: serde_json::json!({
                                "path": "new.txt",
                                "content": "hello"
                            }),
                            name_resolution: aivyx_llm::NameResolution::Known,
                        }],
                        text_so_far: String::new(),
                        usage: LlmUsage::default(),
                    },
                },
                final_step(&["done"], "done"),
            ]
            .into(),
        ),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    let transport = Arc::new(ScriptedTransport::with_queue(vec![IncomingMessage {
        message_id: 1,
        channel_id: 777,
        author_id: 42,
        text: "write a file".to_string(),
    }]));

    let mut config = discord_session_config(Arc::clone(&storage));
    config.tools = Arc::new(ToolRegistry::new(vec![write_tool]));
    config.capabilities = CapabilitySet::from_scopes([
        Scope::parse(&format!("fs.write:{}/**", fs_root.display())).unwrap(),
    ]);

    let shutdown = CancellationToken::new();
    let watcher_shutdown = shutdown.clone();
    let watcher_transport = Arc::clone(&transport);
    tokio::spawn(async move {
        loop {
            if watcher_transport.sent().await.len() >= 1 {
                watcher_shutdown.cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    let checkpointer = Arc::new(
        aivyx_checkpoint::GitCheckpointer::detect(&fs_root, vec![])
            .await
            .expect("fs_root is a real git repo"),
    );

    tokio::time::timeout(
        Duration::from_secs(5),
        run_discord_session_with_transport(
            "aivyx-discord-test",
            Arc::clone(&transport),
            config,
            provider,
            audit,
            Some(checkpointer),
            shutdown,
        ),
    )
    .await
    .expect("run_discord_session_with_transport must exit within the 5s test bound")
    .expect("run_discord_session_with_transport must return Ok");

    let refs = aivyx_checkpoint::test_support::git(
        &fs_root,
        &["for-each-ref", "refs/aivyx/checkpoints/"],
    )
    .await;
    assert_eq!(
        refs.lines().filter(|l| !l.is_empty()).count(),
        1,
        "the dispatched fs.write must produce exactly one checkpoint: {refs}"
    );

    let _ = std::fs::remove_dir_all(&parent);
    let _ = std::fs::remove_dir_all(&fs_root);
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cd /home/julian/Projects/Rust/aivyx && cargo test -p aivyx-discord discord_dispatched_fs_write_produces_a_checkpoint`
Expected: FAIL — `this function takes 6 arguments but 7 arguments were supplied` (the test already passes the new `Some(checkpointer)` argument the current signature doesn't accept yet).

- [ ] **Step 3: Add the parameter to all three functions**

In `crates/aivyx-discord/src/session.rs`, find `pub(crate) async fn run_discord_session_with_mailbox<T>(` (its current parameter list is `channel, config, provider, audit, mut mailbox, shutdown`). Add a new parameter immediately after `audit: Arc<dyn AuditHook>,`:

```rust
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

In that same function's body, find:

```rust
    let agent = ConcreteAgent::new(
        AgentId::new(),
        config.capabilities,
        registry,
        audit,
        move || {
            Box::new(LlmPlanner::new(
                Arc::clone(&provider_for_factory),
                Arc::clone(&registry_for_factory),
                planner_config.clone(),
            ))
        },
    )
    .with_tool_allowlist(config.tool_allowlist)
    .with_memory_topic_prefix(config.memory_topic_prefix);
```

Replace with:

```rust
    let agent = ConcreteAgent::new(
        AgentId::new(),
        config.capabilities,
        registry,
        audit,
        move || {
            Box::new(LlmPlanner::new(
                Arc::clone(&provider_for_factory),
                Arc::clone(&registry_for_factory),
                planner_config.clone(),
            ))
        },
    )
    .with_tool_allowlist(config.tool_allowlist)
    .with_memory_topic_prefix(config.memory_topic_prefix)
    .with_checkpointer(checkpointer);
```

Find `pub(crate) async fn run_discord_session_with_transport<T>(` (current parameter list `channel_name, transport, config, provider, audit, shutdown`). Add the same new parameter immediately after `audit: Arc<dyn AuditHook>,`:

```rust
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

Inside its per-channel spawn block, find:

```rust
            let config_clone = config.clone();
            let provider_clone = Arc::clone(&provider);
            let audit_clone = Arc::clone(&audit);
            let shutdown_clone = shutdown.clone();
            let handle = tokio::spawn(async move {
                run_discord_session_with_mailbox(
                    dchannel,
                    config_clone,
                    provider_clone,
                    audit_clone,
                    rx,
```

Replace with:

```rust
            let config_clone = config.clone();
            let provider_clone = Arc::clone(&provider);
            let audit_clone = Arc::clone(&audit);
            let checkpointer_clone = checkpointer.clone();
            let shutdown_clone = shutdown.clone();
            let handle = tokio::spawn(async move {
                run_discord_session_with_mailbox(
                    dchannel,
                    config_clone,
                    provider_clone,
                    audit_clone,
                    checkpointer_clone,
                    rx,
```

(the line after `rx,` in the original — whatever it is, e.g. `shutdown_clone.clone()` or similar — is unaffected; only the new `checkpointer_clone,` line is inserted between `audit_clone,` and `rx,`).

Find `pub async fn run_discord_session(` (current parameter list `channel_name, token, config, provider, audit, shutdown`). Add the same new parameter immediately after `audit: Arc<dyn AuditHook>,`:

```rust
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

Its body's call to `run_discord_session_with_transport(...)` currently reads:

```rust
    run_discord_session_with_transport(
        channel_name,
        transport,
        config,
        provider,
        audit,
        shutdown,
    )
    .await
```

Replace with:

```rust
    run_discord_session_with_transport(
        channel_name,
        transport,
        config,
        provider,
        audit,
        checkpointer,
        shutdown,
    )
    .await
```

- [ ] **Step 4: Update the test's own imports**

At the top of `crates/aivyx-discord/src/tests.rs`, add (if not already present — check the existing `use` block first):

```rust
use aivyx_llm::{LlmStepEnd, LlmUsage};
use aivyx_core::tools::fs::FsWriteToolConfig;
```

- [ ] **Step 5: Update the one real call site in `aivyx.rs`**

Read the surrounding code first (around line 9656). Find:

```rust
            aivyx_discord::run_discord_session(
                "aivyx-discord",
                token_str,
                discord_config,
                provider,
                audit,
                shutdown,
            )
            .await
            .map(|_report| ())
```

Replace with:

```rust
            aivyx_discord::run_discord_session(
                "aivyx-discord",
                token_str,
                discord_config,
                provider,
                audit,
                checkpointer.clone(),
                shutdown,
            )
            .await
            .map(|_report| ())
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p aivyx-discord discord_dispatched_fs_write_produces_a_checkpoint`
Expected: PASS.

- [ ] **Step 7: Run the full `aivyx-discord` suite, build `aivyx-cli`, and clippy**

Run: `cargo test -p aivyx-discord && cargo build -p aivyx-cli && cargo clippy -p aivyx-discord -p aivyx-cli --all-targets`
Expected: all clean.

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-discord/src/session.rs crates/aivyx-discord/src/tests.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Wire GitCheckpointer into aivyx-discord's session chain

Threads the same checkpointer instance aivyx.rs already builds through
run_discord_session -> run_discord_session_with_transport ->
run_discord_session_with_mailbox, mirroring the aivyx-checkpoint
adoption project's own Task 4 pattern.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `aivyx-slack`

**Files:**
- Modify: `crates/aivyx-slack/src/session.rs` (identical shape to Task 1)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the one real `run_slack_session(...)` call site)
- Test: `crates/aivyx-slack/src/tests.rs`

**Interfaces:**
- Consumes: same as Task 1.
- Produces: `run_slack_session`, `run_slack_session_with_transport`, `run_slack_session_with_mailbox` each gain the same new `checkpointer` parameter.

`aivyx-slack`'s session chain is structurally identical to `aivyx-discord`'s (confirmed during design — same three functions, same per-partition `tokio::spawn` pattern, same `ConcreteAgent::new` shape). Apply the identical edits from Task 1, substituting crate/function/type names:

- [ ] **Step 1: Write the failing test**

In `crates/aivyx-slack/src/tests.rs`, add a test with the same structure as Task 1 Step 1's `discord_dispatched_fs_write_produces_a_checkpoint`, adapted to Slack's own existing test helpers and types (read `crates/aivyx-slack/src/tests.rs` first to find its own `scratch_storage`-equivalent, `slack_session_config`-equivalent, and `ScriptedTransport`/`ScriptedStep`/`ScriptedProvider` definitions — they mirror Discord's by design, per the crate's own doc comments, but confirm the exact local names before writing the test, since a mismatched helper name won't compile). Name it `slack_dispatched_fs_write_produces_a_checkpoint`. Use `IncomingMessage`'s Slack-specific fields (`team_id`/`channel_id` rather than Discord's bare `channel_id`) and pass `Some(checkpointer)` as the new argument to `run_slack_session_with_transport(...)`, in the same position Task 1 used (immediately after `audit`).

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p aivyx-slack slack_dispatched_fs_write_produces_a_checkpoint`
Expected: FAIL — argument-count mismatch, same shape as Task 1 Step 2.

- [ ] **Step 3: Add the parameter to all three functions**

In `crates/aivyx-slack/src/session.rs`:

`run_slack_session_with_mailbox<T>` — add `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,` immediately after `audit: Arc<dyn AuditHook>,` in the signature; add `.with_checkpointer(checkpointer)` to the `ConcreteAgent::new(...)` builder chain (after `.with_memory_topic_prefix(config.memory_topic_prefix)`, mirroring Task 1 Step 3 exactly).

`run_slack_session_with_transport<T>` — add the same new parameter after `audit: Arc<dyn AuditHook>,`; inside the per-partition spawn block, add `let checkpointer_clone = checkpointer.clone();` alongside the existing `config_clone`/`provider_clone`/`audit_clone`/`shutdown_clone` lines, and insert `checkpointer_clone,` into the `run_slack_session_with_mailbox(...)` spawned call, in the same position (immediately after `audit_clone,`).

`run_slack_session` — add the same new parameter after `audit: Arc<dyn AuditHook>,`; insert `checkpointer,` into its call to `run_slack_session_with_transport(...)`, immediately after `audit,`.

- [ ] **Step 4: Update the test's own imports**

Mirror Task 1 Step 4, adapted to whatever `use` block already exists in `crates/aivyx-slack/src/tests.rs`.

- [ ] **Step 5: Update the one real call site in `aivyx.rs`**

Read the surrounding code first (around line 9786). Find:

```rust
            aivyx_slack::run_slack_session(
                "aivyx-slack",
                bot_token_str,
                app_token_str,
                slack_config,
                provider,
                audit,
                shutdown,
            )
            .await
            .map(|_report| ())
```

Replace with:

```rust
            aivyx_slack::run_slack_session(
                "aivyx-slack",
                bot_token_str,
                app_token_str,
                slack_config,
                provider,
                audit,
                checkpointer.clone(),
                shutdown,
            )
            .await
            .map(|_report| ())
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p aivyx-slack slack_dispatched_fs_write_produces_a_checkpoint`
Expected: PASS.

- [ ] **Step 7: Run the full `aivyx-slack` suite, build `aivyx-cli`, and clippy**

Run: `cargo test -p aivyx-slack && cargo build -p aivyx-cli && cargo clippy -p aivyx-slack -p aivyx-cli --all-targets`
Expected: all clean.

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-slack/src/session.rs crates/aivyx-slack/src/tests.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Wire GitCheckpointer into aivyx-slack's session chain

Same shape as the aivyx-discord fix (Task 1 of this plan) and the
aivyx-checkpoint adoption project's own Task 4 pattern.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: `aivyx-telegram`

**Files:**
- Modify: `crates/aivyx-telegram/src/session.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the one real `run_telegram_multi_session(...)` call site)
- Test: `crates/aivyx-telegram/src/tests.rs`

**Interfaces:**
- Consumes: same as Tasks 1-2.
- Produces: `run_telegram_multi_session`, `run_telegram_multi_session_with_transport`, `run_telegram_session_with_mailbox` (note: the innermost function is named for the *singular* session even though it's used by the *multi* chain — confirmed during design; this is the real, existing name, not a naming inconsistency this plan introduces) each gain the same new `checkpointer` parameter.

**Do not touch `run_telegram_session` (singular, line ~354) or anything only reachable through it** — it has zero real callers, confirmed via a full-workspace grep during design.

- [ ] **Step 1: Write the failing test**

In `crates/aivyx-telegram/src/tests.rs`, add a test with the same structure as Task 1's, adapted to Telegram's own existing test helpers (read the file first — it already has a `ScriptedProvider`/`ScriptedStep` pair per the `run_telegram_session_two_chats_persistent_e2e` test read during design, which is the closest prior art for the `LlmStepEnd::ToolCalls { calls: vec![aivyx_llm::ToolCallEnd { ... }], ... }` shape this new test also needs). Name it `telegram_dispatched_fs_write_produces_a_checkpoint`. `run_telegram_multi_session_with_transport`'s real signature (found during design) is:

```rust
pub(crate) async fn run_telegram_multi_session_with_transport<T>(
    channel_name: impl Into<String> + Clone,
    transport: Arc<T>,
    chat_filter: Option<i64>,
    config: TelegramSessionConfig,
    provider: Arc<dyn LlmProvider>,
    audit: Arc<dyn AuditHook>,
    long_poll_timeout_secs: u32,
    shutdown: CancellationToken,
) -> Result<TelegramMultiSessionReport, String>
```

The new test's call to it must supply `chat_filter: None` (accept every chat, matching the existing `discord_session_smoke_e2e`-style tests' permissiveness) and pass `Some(checkpointer)` as the new argument, positioned immediately after `audit` (matching Tasks 1-2's convention) and before `long_poll_timeout_secs`.

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p aivyx-telegram telegram_dispatched_fs_write_produces_a_checkpoint`
Expected: FAIL — argument-count mismatch.

- [ ] **Step 3: Add the parameter to all three functions**

In `crates/aivyx-telegram/src/session.rs`:

`run_telegram_session_with_mailbox<T>` (the innermost, real construction site — current parameter list `channel, config, provider, audit, mut mailbox, shutdown`) — add `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,` immediately after `audit: Arc<dyn AuditHook>,`; add `.with_checkpointer(checkpointer)` to the `ConcreteAgent::new(...)` builder chain, mirroring Tasks 1-2's exact insertion point.

`run_telegram_multi_session_with_transport<T>` (current parameter list `channel_name, transport, chat_filter, config, provider, audit, long_poll_timeout_secs, shutdown`) — add the same new parameter immediately after `audit: Arc<dyn AuditHook>,` (before `long_poll_timeout_secs: u32,`). Inside its per-chat spawn block, add `let checkpointer_clone = checkpointer.clone();` alongside the existing clones, and insert `checkpointer_clone,` into the `run_telegram_session_with_mailbox(...)` spawned call, immediately after `audit_clone,`.

`run_telegram_multi_session` (current parameter list `channel_name, token, chat_filter, config, provider, audit, shutdown`) — add the same new parameter after `audit: Arc<dyn AuditHook>,`. Its body's call to `run_telegram_multi_session_with_transport(...)` currently reads:

```rust
    run_telegram_multi_session_with_transport(
        channel_name,
        transport,
        chat_filter,
        config,
        provider,
        audit,
        LONG_POLL_TIMEOUT_SECS,
        shutdown,
    )
    .await
```

Replace with:

```rust
    run_telegram_multi_session_with_transport(
        channel_name,
        transport,
        chat_filter,
        config,
        provider,
        audit,
        checkpointer,
        LONG_POLL_TIMEOUT_SECS,
        shutdown,
    )
    .await
```

- [ ] **Step 4: Update the test's own imports**

Mirror Task 1 Step 4, adapted to `crates/aivyx-telegram/src/tests.rs`'s own existing `use` block.

- [ ] **Step 5: Update the one real call site in `aivyx.rs`**

Read the surrounding code first (around line 9551). Find:

```rust
            run_telegram_multi_session(
                "aivyx-telegram",
                token_str,
                chat_filter,
                telegram_config,
                provider,
                audit,
                shutdown,
            )
            .await
            .map(|_report| ())
```

Replace with:

```rust
            run_telegram_multi_session(
                "aivyx-telegram",
                token_str,
                chat_filter,
                telegram_config,
                provider,
                audit,
                checkpointer.clone(),
                shutdown,
            )
            .await
            .map(|_report| ())
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p aivyx-telegram telegram_dispatched_fs_write_produces_a_checkpoint`
Expected: PASS.

- [ ] **Step 7: Run the full `aivyx-telegram` suite, build `aivyx-cli`, and clippy**

Run: `cargo test -p aivyx-telegram && cargo build -p aivyx-cli && cargo clippy -p aivyx-telegram -p aivyx-cli --all-targets`
Expected: all clean.

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-telegram/src/session.rs crates/aivyx-telegram/src/tests.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Wire GitCheckpointer into aivyx-telegram's multi-session chain

Same shape as Tasks 1-2 of this plan. run_telegram_session (singular)
has zero real callers anywhere in the workspace and is untouched.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: `aivyx-team`'s `SpecialistFactory` + the daemon mission path

**Files:**
- Modify: `crates/aivyx-team/src/factory.rs` (`SpecialistFactory` field + builder + `build`'s `ConcreteAgent::new` call)
- Modify: `crates/aivyx-team/src/assembly.rs` (`TeamAssembly::build`'s new parameter)
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs` (`TeamRunDeps` new field + its `TeamAssembly::build` call site)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the `TeamRunDeps { ... }` literal in the daemon's "Chapter L" team-mission block)
- Test: `crates/aivyx-team/src/factory.rs`'s own `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: same `checkpointer` binding as every other task.
- Produces: `SpecialistFactory::with_checkpointer(self, checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>) -> Self`; `TeamAssembly::build(...)` gains a new `checkpointer` parameter (Task 5 also calls this function and needs to know its new signature).

This task covers only the daemon's own team-mission path (`team_mission_driver.rs`). The CLI's separate `team run` command (`aivyx_modules/team.rs::run_mission`, which also calls `TeamAssembly::build` and needs updating for the same signature change) is Task 5.

- [ ] **Step 1: Write the failing test**

In `crates/aivyx-team/src/factory.rs`'s `#[cfg(test)] mod tests` block, add (near the existing `.build(...)`-exercising tests, reusing the module's own `factory(base)` helper and `member(...)` helper already used by those tests — read the surrounding test code first to confirm their exact current signatures before calling them):

```rust
    #[test]
    fn build_attaches_the_checkpointer_when_configured() {
        // The checkpointer is opaque to this test — SpecialistFactory only
        // needs to plumb whatever Option it's given through to
        // ConcreteAgent::with_checkpointer, which Task 2 of the
        // aivyx-checkpoint adoption plan already tested end-to-end against
        // a real GitCheckpointer. Passing None here and confirming build()
        // still succeeds is sufficient to prove the new field/builder
        // don't break construction — the wiring itself is exercised by
        // this crate's real callers (team_mission_driver.rs / run_mission),
        // not by fabricating a redundant real-git fixture in this crate.
        let f = factory(vec![Arc::new(FakeTool::new("fs.read"))])
            .with_checkpointer(None);
        let lead = member("spec", &["fs.read"], &[]);
        let result = f.build(&lead, &lead_caps_for(&lead));
        assert!(result.is_ok(), "build must still succeed with no checkpointer configured");
    }
```

Before writing this, read `crates/aivyx-team/src/factory.rs`'s existing test module in full to find the real names of `FakeTool`, `member`, and however the existing tests derive `lead_caps` for a call to `.build(...)` (the sketch above uses placeholder helper names — `FakeTool::new`, `lead_caps_for` — that must be replaced with whatever this file's own tests actually call; do not invent new helper names if equivalent ones already exist).

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cd /home/julian/Projects/Rust/aivyx && cargo test -p aivyx-team build_attaches_the_checkpointer_when_configured`
Expected: FAIL — `no method named with_checkpointer found for struct SpecialistFactory`.

- [ ] **Step 3: Add the field, builder, and wiring to `SpecialistFactory`**

In `crates/aivyx-team/src/factory.rs`, find `pub struct SpecialistFactory {` (its current field list ends with `member_backends: std::collections::HashMap<String, SpecialistBackend>,`). Add, after that field:

```rust
    /// `aivyx-checkpoint` — attached to every built specialist so an
    /// fs_root-mutating tool call it makes gets checkpointed, same as the
    /// lead agent. `None` (the default) preserves pre-checkpoint behavior.
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

In `SpecialistFactory::new`, find the returned struct literal's last field, `member_backends: std::collections::HashMap::new(),`. Add, after it:

```rust
            checkpointer: None,
```

Immediately after the existing `pub fn with_dialogue(mut self, bus: Arc<MessageBus>, dialogue: DialogueConfig) -> Self { ... }` method, add:

```rust

    /// Attach an `aivyx-checkpoint` `GitCheckpointer` to every specialist
    /// this factory builds. `None` means "no checkpointer" (checkpointing
    /// disabled, or `fs_root` isn't a git repository), preserving
    /// pre-checkpoint behavior — same shape as `ConcreteAgent::with_checkpointer`.
    pub fn with_checkpointer(
        mut self,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    ) -> Self {
        self.checkpointer = checkpointer;
        self
    }
```

In `SpecialistFactory::build`, find:

```rust
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            Arc::clone(&self.audit),
            move || {
                let cfg = LlmPlannerConfig::new(&model)
                    .with_system_prompt(&soul)
                    .with_max_tokens(max_tokens);
                Box::new(LlmPlanner::new(
                    Arc::clone(&provider),
                    Arc::clone(&registry_for_planner),
                    cfg,
                ))
            },
        );
```

Replace with:

```rust
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            Arc::clone(&self.audit),
            move || {
                let cfg = LlmPlannerConfig::new(&model)
                    .with_system_prompt(&soul)
                    .with_max_tokens(max_tokens);
                Box::new(LlmPlanner::new(
                    Arc::clone(&provider),
                    Arc::clone(&registry_for_planner),
                    cfg,
                ))
            },
        )
        .with_checkpointer(self.checkpointer.clone());
```

(the following line, `Ok(TurnSafety::autonomous().apply(agent))`, is unaffected — `agent` is now the value produced by the extended builder chain instead of the bare `ConcreteAgent::new(...)` call.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-team build_attaches_the_checkpointer_when_configured`
Expected: PASS.

- [ ] **Step 5: Thread it through `TeamAssembly::build`**

In `crates/aivyx-team/src/assembly.rs`, find `TeamAssembly::build`'s signature (currently ending `member_backends: std::collections::HashMap<String, crate::factory::SpecialistBackend>,`). Add a new parameter after it:

```rust
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

Find:

```rust
        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends);
```

Replace with:

```rust
        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends)
            .with_checkpointer(checkpointer);
```

- [ ] **Step 6: Verify `aivyx-team` builds (its own test callers of `TeamAssembly::build` will now fail to compile)**

Run: `cargo build -p aivyx-team 2>&1 | head -5 && cargo test -p aivyx-team --no-run 2>&1 | grep "error\[" | head -20`
Expected: the library builds clean; the test binary fails with `E0061` errors at every existing `TeamAssembly::build(...)` call inside `crates/aivyx-team/src/assembly.rs`'s own `#[cfg(test)] mod tests` (there are 3, found during design — at approximately lines 178, 195, 258 of that file). Add `None` as the new argument, in the new parameter's position, at each of those 3 call sites — read the surrounding code first to place it correctly relative to each call's own argument list (which may differ slightly test-to-test). Re-run `cargo test -p aivyx-team --no-run` until it compiles clean.

- [ ] **Step 7: Add the field to `TeamRunDeps` and wire the daemon's mission-driver call site**

In `crates/aivyx-channel/src/team_mission_driver.rs`, find `pub struct TeamRunDeps {` (its current field list ends with `pub file_root: ...` or similar — read the actual current last field before editing, since the design's own read of this struct was truncated; add the new field as the struct's last member regardless of exactly what precedes it). Add:

```rust
    /// `aivyx-checkpoint` — passed through to every specialist's
    /// `SpecialistFactory` so fs_root-mutating tool calls made during a team
    /// mission are checkpointed, same as every other agent construction path.
    pub checkpointer: Option<std::sync::Arc<aivyx_core::GitCheckpointer>>,
```

Find the function containing `let assembly = TeamAssembly::build(` (around line 983, the function that takes `deps: &TeamRunDeps` and `mut config: TeamConfig`). Its current call reads:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        lead_caps,
        member_backends,
    )?;
```

Replace with:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        lead_caps,
        member_backends,
        deps.checkpointer.clone(),
    )?;
```

- [ ] **Step 8: Update `aivyx.rs`'s `TeamRunDeps { ... }` literal**

Read the surrounding code first (around line 8659, the "Chapter L (L.5)" team-mission block, well after `checkpointer` is built near `canonical_root`). Add, as a new field in the struct literal (alongside `provider:`/`model:`/`audit:`/etc.):

```rust
                checkpointer: checkpointer.clone(),
```

- [ ] **Step 9: Run the full `aivyx-team`/`aivyx-channel` suites, build `aivyx-cli`, and clippy**

Run: `cargo test -p aivyx-team -p aivyx-channel && cargo build -p aivyx-cli && cargo clippy -p aivyx-team -p aivyx-channel -p aivyx-cli --all-targets`
Expected: all clean.

- [ ] **Step 10: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-team/src/factory.rs crates/aivyx-team/src/assembly.rs \
  crates/aivyx-channel/src/team_mission_driver.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Wire GitCheckpointer into aivyx-team's daemon mission path

SpecialistFactory gains a checkpointer field/builder (mirroring
with_dialogue's exact shape), threaded through TeamAssembly::build's
new parameter and TeamRunDeps' new field to the daemon's real
team-mission driver. The CLI's own separate 'team run' command
(aivyx_modules/team.rs::run_mission) is a separate task.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5: `aivyx_modules/team.rs::run_mission` — the CLI's own `team run` command

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team.rs` (`run_mission`'s signature, its `TeamAssembly::build` call, and its own lead-agent `ConcreteAgent::new` call)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the one real `team::run_mission(...)` call site)

**Interfaces:**
- Consumes: `TeamAssembly::build`'s new signature (Task 4) — this task's `TeamAssembly::build(...)` call must supply the new `checkpointer` argument Task 4 added. Also consumes the same `checkpointer` binding every other task in this plan uses.
- Produces: `run_mission` gains a new `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>` parameter.

`run_mission` has two separate agent-construction points: its own call to `TeamAssembly::build(...)` (for the mission's specialists — now requiring Task 4's new parameter) and its own lead agent's `ConcreteAgent::new(...)` (a completely separate construction, not going through `SpecialistFactory` at all, since the lead is orchestration-only).

- [ ] **Step 1: Add the parameter and wire both construction points**

Read `crates/aivyx-cli/src/bin/aivyx_modules/team.rs` in full first — `run_mission`'s current signature is:

```rust
pub async fn run_mission(
    provider: Arc<dyn LlmProvider>,
    model: &str,
    max_tokens: u32,
    audit: Arc<dyn AuditHook>,
    base_tools: Vec<Arc<dyn Tool>>,
    mission: &str,
    config: Option<&str>,
) -> Result<(), String> {
```

Add a new parameter immediately after `audit: Arc<dyn AuditHook>,`:

```rust
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

Its current call to `TeamAssembly::build(...)` reads:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&provider),
        model,
        max_tokens,
        Arc::clone(&audit),
        base_tools,
        lead_caps.clone(),
        std::collections::HashMap::new(),
    )
    .map_err(|e| format!("failed to assemble team: {e}"))?;
```

Replace with:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&provider),
        model,
        max_tokens,
        Arc::clone(&audit),
        base_tools,
        lead_caps.clone(),
        std::collections::HashMap::new(),
        checkpointer.clone(),
    )
    .map_err(|e| format!("failed to assemble team: {e}"))?;
```

Further down in the same function, its own lead-agent construction reads:

```rust
    let agent = ConcreteAgent::new(
        AgentId::new(),
        lead_caps,
        registry,
        audit,
        move || {
            let cfg = LlmPlannerConfig::new(&model_owned)
                .with_system_prompt(&soul)
                .with_max_tokens(max_tokens);
```

Read further to find the end of this builder chain (the closing of the `ConcreteAgent::new(...)` call and any `.with_...` methods already chained onto it) before editing — add `.with_checkpointer(checkpointer)` as the last method in that chain, after whatever is already there.

- [ ] **Step 2: Update the one real call site in `aivyx.rs`**

Read the surrounding code first (around line 7693). Find:

```rust
        return team::run_mission(
```

and its full argument list. Add `checkpointer.clone(),` as a new argument, in the same relative position used by every other task in this plan (immediately after the `audit`-equivalent argument).

**Scope verification (do this before assuming the edit above compiles):** run `cargo build -p aivyx-cli` after making the edit. If `checkpointer` is reported as not found in scope at this call site, that means an early return or conditional branch between `checkpointer`'s own construction (~line 6001) and this call site (~line 7693) removed it from scope — locate that branch, and fix it the same way Task 3 of the aivyx-checkpoint adoption project fixed `child_agent`'s closure-capture issue: clone `checkpointer` into an earlier, appropriately-named binding (e.g. `checkpointer_for_team`) before whatever narrows scope, and use that clone here instead. Do not restructure `run()`'s own control flow to work around this.

- [ ] **Step 3: Run the full `aivyx-cli` suite and clippy**

Run: `cargo test -p aivyx-cli && cargo clippy -p aivyx-cli --all-targets`
Expected: clean.

- [ ] **Step 4: Run the complete workspace suite one final time**

Run: `cd /home/julian/Projects/Rust/aivyx && cargo test --workspace --exclude aivyx-desktop 2>&1 | grep -E "test result|FAILED"`
Expected: every crate's count matches its pre-Task-1 baseline plus the 4 new tests added across Tasks 1-4 (discord, slack, telegram, team — one each), zero failures, no `FAILED` lines.

- [ ] **Step 5: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-cli/src/bin/aivyx_modules/team.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Wire GitCheckpointer into the CLI's own 'team run' command

run_mission's two construction points (its own lead agent, and the
specialists built via TeamAssembly::build) both now receive the same
checkpointer instance every other path in the workspace shares. This
closes the last of the 5 real construction sites this plan set out to
wire — every real ConcreteAgent construction site in aivyx now has
checkpoint protection.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Self-review notes

**Spec coverage:** the design doc's 5 numbered sites map onto Tasks 1-5
exactly (discord → Task 1, slack → Task 2, telegram → Task 3,
`SpecialistFactory`/daemon mission path → Task 4, CLI `run_mission` →
Task 5). The design's own flagged open question (whether `checkpointer`
is genuinely in scope at `run_mission`'s call site) is carried into Task
5 Step 2 verbatim, with the same documented fallback the design
specified, rather than resolved by assumption in either direction.

**Placeholder scan:** Tasks 1-3 give complete, real code transcribed
from files read during brainstorming for every signature and call-site
edit. Task 2's test step deliberately delegates exact helper names to
"read the file first" rather than inventing them, since Slack's test
module wasn't read line-by-line during brainstorming the way Discord's
was — this is not a placeholder in the prohibited sense (it doesn't
describe what to do without how; it gives the exact target shape, exact
new test name, and exact adaptation instructions) but is flagged here so
the self-review is honest about which task has slightly less verified
detail than its siblings. Task 4's own test step similarly asks the
implementer to confirm real helper names against the file before use,
for the same reason (the file's existing test module wasn't read in
full during brainstorming, only the `build()` method and one `.build(...)`
call site were).

**Type consistency:** `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>`
is the exact type used at every new parameter and field across all 5
tasks — checked signature-by-signature against Task 2 of the prior
`aivyx-checkpoint` adoption project's own `ConcreteAgent::with_checkpointer`
(the type this whole plan threads outward from). `TeamAssembly::build`'s
new parameter (Task 4 Step 5) is consumed identically by both of its
real callers (Task 4 Step 7's `team_mission_driver.rs` edit, Task 5
Step 1's `run_mission` edit) — the same signature, same argument
position (last parameter), in both.

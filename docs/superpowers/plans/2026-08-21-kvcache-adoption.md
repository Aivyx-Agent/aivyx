# `aivyx-kvcache` Adoption in `aivyx` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `aivyx` persist and restore local-model KV-cache state across process restarts when talking to `llama-server` (`[agent] provider = "llama_cpp"`), using the same `aivyx-kvcache` crate `aivyx-coder` already adopted — opt-in, zero behavior change for every other provider.

**Architecture:** `aivyx`'s architecture differs from `aivyx-coder`'s in one load-bearing way this plan is built around: there is no persistent, cross-turn `Agent` object to hang a `Drop`-based release on. `ConcreteAgent::turn()` constructs a **fresh `LlmPlanner` on every single turn** via a stored `planner_factory: Box<dyn Fn() -> Box<dyn TurnPlanner>>` closure (`crates/aivyx-core/src/agent.rs:362`), and that `LlmPlanner` is a local variable dropped when `turn()` returns. So the checkout/warm-up/restore/pin/release policy lives on `LlmPlanner` itself (not a longer-lived type), scoped to one turn's lifetime — checkout happens in `begin_turn` (the `TurnPlanner` trait's own per-turn entry point), release happens via `impl Drop for LlmPlanner` firing naturally when the turn ends. Two independent wiring sites need the shared `KvSlotPool`/`LlamaServerSlotStore`: the daemon's own main-agent `planner_factory` closure, and `aivyx-team`'s `SpecialistFactory` (every Nonagon specialist/lead also gets a fresh per-turn `LlmPlanner` from its own closure).

**Tech Stack:** Rust, `aivyx-kvcache` (new git dependency, pinned rev `e1b06c9960ee98841d9b91978a11dd99ed388490` — confirmed current `master` HEAD at plan-writing time), no other new dependencies.

## Global Constraints

- `[agent] provider = "llama_cpp"` (`ProviderKind::LlamaCpp`, already shipped, Phase 133 — `crates/aivyx-config/src/lib.rs:393-400`) is the gate. No new config field needed here, unlike `aivyx-coder`.
- **Unlike `aivyx-coder`, `base_url` is already the bare origin here** — verified against real code: `crates/aivyx-cli/src/bin/aivyx.rs:5851`'s `DEFAULT_LLAMACPP_BASE_URL = "http://localhost:8080"` has no `/v1` suffix, and `OpenAiProvider`'s own URL construction (`crates/aivyx-llm/src/openai/provider.rs:132`, `format!("{base}/v1/chat/completions")`) appends `/v1/chat/completions` itself. Do **not** add any `/v1`-stripping logic — use the `base_url` value directly for both the `/props` probe and `LlamaServerSlotStore::open`.
- Every kvcache operation is fail-open: a `--slot-save-path`-less server, a network error, a full pool — none of it may ever surface as a turn failure. Log at `warn`, proceed as if kvcache weren't configured.
- The saved/restored KV state must never include real conversation content. `aivyx`'s own `begin_turn` can seed real prior conversation into `history` via an injected `ConversationSeeder` (`crates/aivyx-core/src/llm_planner.rs:301-306`, "so conversational channels see real multi-turn context") — the kvcache warm-up request must be built independently of `self.history`/`conversation_seeder` entirely, using only `config.system_prompt` + `self.tools`, exactly as carefully as `aivyx-coder`'s own `system_prompt_text()` avoided `self.history`. `aivyx` has no repo-map concept (confirmed: no `repo_map`-equivalent field on `LlmPlannerConfig` or `LlmPlanner`), so the stable prefix here is narrower: system prompt + tool defs only.
- The warm-up request needs a trailing placeholder empty `User`-role message — confirmed live on `aivyx-coder`'s own adoption that a system-message-only request is rejected outright by real chat templates (`400`, "No user query found in messages"). Do not write this without the placeholder and rediscover the failure live.
- `id_slot` is pinned on every real request of a turn once a slot is checked out, not just the warm-up call.
- `restore_into_slot`'s `Ok(false)` (a rejected restore) must fall through to the warm-up-and-save path, not be silently discarded — `aivyx-coder`'s own final review found this exact bug (a stuck-cold key with no repair path) and fixed it; build it in correctly from the start here.
- The `/props`-probing HTTP client must have an explicit timeout — `aivyx-coder`'s own fix wave found the hard way that an unbounded client here is a real, not hypothetical, startup-hang risk.
- `CacheMeta.size_bytes` stays a placeholder (`1`) at every call site in this repo — `aivyx-kvcache`'s own `save_from_slot` (this pinned rev) already measures the real saved file's size itself and falls back gracefully when it can't. Do not invent a different workaround here.
- The `aivyx-kvcache` API surface used here (verified against real source, rev `e1b06c9960ee98841d9b91978a11dd99ed388490`) — identical to what `aivyx-coder`'s own plan already documented and this plan re-verified unchanged:
  - `LlamaServerSlotStore::open(store_path: impl Into<PathBuf>, base_url: impl Into<String>, max_bytes: u64) -> Result<Self, KvCacheError>`
  - `LlamaServerSlotStore::save_from_slot(&self, key: &CacheKey, slot_id: u32, meta: CacheMeta) -> Result<(), KvCacheError>`
  - `LlamaServerSlotStore::restore_into_slot(&self, key: &CacheKey, slot_id: u32) -> Result<bool, KvCacheError>` (never `Err` on a plain cache miss)
  - `KvCacheStore::find(&self, key: &CacheKey) -> Result<Option<CacheHandle>, KvCacheError>` (trait method — `use aivyx_kvcache::KvCacheStore;`)
  - `CacheKey { backend_id: String, model_id: String, build_hash: String, prefix_hash: String }`, `CacheMeta { size_bytes: u64, token_count: u64 }`

---

### Task 1: Thread `id_slot` through `LlmRequest` and the OpenAI-compat wire body

**Files:**
- Modify: `crates/aivyx-llm/src/lib.rs`
- Modify: `crates/aivyx-llm/src/openai/provider.rs`

**Interfaces:**
- Consumes: `LlmRequest<'a>` (`crates/aivyx-llm/src/lib.rs:331-357`), `build_request_body` (`crates/aivyx-llm/src/openai/provider.rs:236-301`).
- Produces: `LlmRequest.id_slot: Option<u32>` (new field, owned — no lifetime issue in an otherwise-borrowed struct). `None` (every non-llama-server request, and every llama-server request before checkout) omits the field from the wire body entirely, matching `aivyx-coder`'s own precedent.

- [ ] **Step 1: Write the failing test**

Add to `crates/aivyx-llm/src/openai/provider.rs`'s existing `#[cfg(test)]` module (it already builds `LlmRequest` literals for other `build_request_body` tests — copy that pattern):

```rust
    #[test]
    fn build_request_body_omits_id_slot_when_none() {
        let messages: Vec<LlmMessage> = vec![];
        let tools: Vec<LlmToolDescriptor> = vec![];
        let request = LlmRequest {
            model: "test-model",
            system: None,
            messages: &messages,
            tools: &tools,
            max_tokens: 100,
            temperature: None,
            id_slot: None,
        };
        let body = build_request_body(&request, true, false).unwrap();
        assert!(body.get("id_slot").is_none(), "id_slot must be omitted entirely when None");
    }

    #[test]
    fn build_request_body_includes_id_slot_when_set() {
        let messages: Vec<LlmMessage> = vec![];
        let tools: Vec<LlmToolDescriptor> = vec![];
        let request = LlmRequest {
            model: "test-model",
            system: None,
            messages: &messages,
            tools: &tools,
            max_tokens: 100,
            temperature: None,
            id_slot: Some(2),
        };
        let body = build_request_body(&request, true, false).unwrap();
        assert_eq!(body["id_slot"], serde_json::json!(2));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-llm build_request_body_.*id_slot -- --test-threads=1`
Expected: FAIL to compile — `LlmRequest` has no `id_slot` field yet.

- [ ] **Step 3: Add the field**

In `crates/aivyx-llm/src/lib.rs`, add to `LlmRequest<'a>` (after `temperature`, `crates/aivyx-llm/src/lib.rs:356`):

```rust
    pub temperature: Option<f32>,

    /// llama-server-only: pins this request to a specific `/slots` id (an
    /// extension beyond the OpenAI spec, but honored by llama-server on
    /// `/v1/chat/completions` -- verified empirically against a real
    /// server during `aivyx-coder`'s own kvcache adoption, not documented
    /// in llama-server's own API reference). Only ever set when
    /// `[agent] provider = "llama_cpp"` and a slot has been checked out
    /// (see `LlmPlanner`'s kvcache fields); `None` for every other
    /// provider and every llama-server request before checkout.
    pub id_slot: Option<u32>,
```

In `crates/aivyx-llm/src/openai/provider.rs`'s `build_request_body` (`crates/aivyx-llm/src/openai/provider.rs:258-263`), add to the `body = json!({...})` literal:

```rust
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });

    if let Some(slot_id) = request.id_slot {
        body["id_slot"] = json!(slot_id);
    }
```

- [ ] **Step 4: Fix every other `LlmRequest { ... }` construction site**

Every existing exhaustive `LlmRequest { ... }` literal in the workspace now needs `id_slot: None,` added (any that already go through a helper/constructor rather than a literal don't need changes). Find them:

```bash
grep -rn "LlmRequest {" crates/*/src/**/*.rs crates/*/src/*.rs 2>/dev/null
```

Add `id_slot: None,` to each real (non-test) literal found (the one inside `crates/aivyx-core/src/llm_planner.rs`'s `one_step`, `crates/aivyx-llm/src/lib.rs`'s own fake-provider test helper if it constructs one, and any in `aivyx-llm/src/anthropic/provider.rs`/`ollama/provider.rs` if they pattern-match or reconstruct the struct). Test-only literals (inside `#[cfg(test)]` modules elsewhere in the workspace) also need the field added to compile — add `id_slot: None,` to each; do not change any test's assertions.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo build --workspace` (confirms every call site compiles), then `cargo test -p aivyx-llm -- --test-threads=1`.
Expected: builds clean; the two new tests pass; no regressions.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: thread id_slot through LlmRequest and the OpenAI-compat wire body"
```

---

### Task 2: `KvSlotPool` — a pure numeric checkout/release pool

**Files:**
- Create: `crates/aivyx-llm/src/kv_slot_pool.rs`
- Modify: `crates/aivyx-llm/src/lib.rs` (add `mod kv_slot_pool; pub use kv_slot_pool::KvSlotPool;`)

**Interfaces:**
- Produces: `pub struct KvSlotPool` with `pub fn new(total_slots: u32) -> Self`, `pub fn checkout(&self) -> Option<u32>`, `pub fn release(&self, slot_id: u32)`. Thread-safe (`Mutex`-backed) — later tasks share one instance via `Arc<KvSlotPool>` across every concurrent turn in a process (the daemon's own main agent, plus every concurrent Nonagon specialist/lead turn).
- Not shared code with `aivyx-coder`'s own `KvSlotPool` (a completely separate crate in a separate repo) — a fresh, parallel implementation, identical in shape by design (the same proven pattern), verified independently here.

- [ ] **Step 1: Write the failing tests**

Create `crates/aivyx-llm/src/kv_slot_pool.rs`:

```rust
//! A pure numeric slot-id pool -- tracks which of `0..total_slots` are
//! currently checked out. No I/O, no knowledge of `aivyx-kvcache` at all;
//! `LlmPlanner` (the caller) decides what a checked-out slot id is
//! actually used for.

use std::collections::HashSet;
use std::sync::Mutex;

pub struct KvSlotPool {
    total_slots: u32,
    checked_out: Mutex<HashSet<u32>>,
}

impl KvSlotPool {
    pub fn new(total_slots: u32) -> Self {
        Self {
            total_slots,
            checked_out: Mutex::new(HashSet::new()),
        }
    }

    /// Returns the lowest-numbered free slot id, or `None` if every slot
    /// is already checked out.
    pub fn checkout(&self) -> Option<u32> {
        let mut checked_out = self.checked_out.lock().unwrap();
        (0..self.total_slots).find(|id| checked_out.insert(*id))
    }

    /// Returns `slot_id` to the pool. A `slot_id` that was never checked
    /// out (or already released) is a silent no-op -- release is called
    /// from `Drop` impls, where panicking or erroring is not an option.
    pub fn release(&self, slot_id: u32) {
        self.checked_out.lock().unwrap().remove(&slot_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_returns_lowest_free_id_first() {
        let pool = KvSlotPool::new(4);
        assert_eq!(pool.checkout(), Some(0));
        assert_eq!(pool.checkout(), Some(1));
    }

    #[test]
    fn checkout_returns_none_once_the_pool_is_full() {
        let pool = KvSlotPool::new(2);
        assert_eq!(pool.checkout(), Some(0));
        assert_eq!(pool.checkout(), Some(1));
        assert_eq!(pool.checkout(), None, "pool of size 2 must reject a third concurrent checkout");
    }

    #[test]
    fn release_makes_a_slot_available_again() {
        let pool = KvSlotPool::new(1);
        let id = pool.checkout().expect("pool of size 1 has a free slot");
        assert_eq!(pool.checkout(), None, "the only slot is already checked out");
        pool.release(id);
        assert_eq!(pool.checkout(), Some(id), "release must make the slot checkoutable again");
    }

    #[test]
    fn releasing_a_never_checked_out_id_is_a_silent_no_op() {
        let pool = KvSlotPool::new(4);
        pool.release(99); // never checked out -- must not panic
        assert_eq!(pool.checkout(), Some(0), "pool must still function normally after a no-op release");
    }
}
```

(This is already written using `Iterator::find` from the start, unlike `aivyx-coder`'s own first draft, which picked up a clippy `manual_find` lint it had to fix in a later pass — no need to repeat that here.)

- [ ] **Step 2: Wire the module in and run the tests**

Add to `crates/aivyx-llm/src/lib.rs`:

```rust
mod kv_slot_pool;
pub use kv_slot_pool::KvSlotPool;
```

Run: `cargo test -p aivyx-llm kv_slot_pool -- --test-threads=1`
Expected: PASS, 4/4 new tests.

- [ ] **Step 3: Commit**

```bash
git add crates/aivyx-llm/src/kv_slot_pool.rs crates/aivyx-llm/src/lib.rs
git commit -m "feat: add KvSlotPool, a pure numeric slot checkout/release pool"
```

---

### Task 3: `/props` probe — `total_slots` + `build_info`

**Files:**
- Create: `crates/aivyx-llm/src/kvcache_probe.rs`
- Modify: `crates/aivyx-llm/src/lib.rs` (add `mod kvcache_probe; pub use kvcache_probe::{fetch_llama_slots_info, LlamaSlotsInfo, KVCACHE_PROBE_TIMEOUT};`)

**Interfaces:**
- Produces: `pub struct LlamaSlotsInfo { pub total_slots: u32, pub build_info: String }`, `pub fn parse_llama_slots_info(json: &serde_json::Value) -> Option<LlamaSlotsInfo>` (pure parser, unit-testable without a real server), and `pub async fn fetch_llama_slots_info(base_url: &str) -> Option<LlamaSlotsInfo>` (does the real, timeout-bounded HTTP fetch + parse in one call — `aivyx` has no existing `/props` probe to extend, unlike `aivyx-coder`, so this task builds the whole thing fresh, fetch included). `pub const KVCACHE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);` — the bound the fail-open Global Constraint depends on.

- [ ] **Step 1: Write the failing tests**

Create `crates/aivyx-llm/src/kvcache_probe.rs`:

```rust
//! Fetches `total_slots` + `build_info` from a real llama-server `/props`
//! response -- the one piece of server metadata `LlmPlanner`'s kvcache
//! wiring needs at startup to size the `KvSlotPool` and detect a server
//! upgrade (via `build_info`, folded into `CacheKey.build_hash`).
//!
//! `aivyx` has no other `/props`-consuming code today (unlike
//! `aivyx-coder`, which already probes `/props` for context-window
//! detection) -- this is a standalone fetch, not an extension of an
//! existing one.

use std::time::Duration;

/// Bounds the `/props` fetch below -- an unbounded client here was a
/// real (not hypothetical) startup-hang risk found the hard way during
/// `aivyx-coder`'s own kvcache adoption: a server that accepts the TCP
/// connection but doesn't answer until a large model finishes loading
/// blocks forever without one.
pub const KVCACHE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlamaSlotsInfo {
    pub total_slots: u32,
    pub build_info: String,
}

/// Pure parser over an already-fetched `/props` JSON body -- no I/O,
/// independently testable without a real server.
pub fn parse_llama_slots_info(json: &serde_json::Value) -> Option<LlamaSlotsInfo> {
    let total_slots = json.get("total_slots")?.as_u64()? as u32;
    let build_info = json.get("build_info")?.as_str()?.to_string();
    Some(LlamaSlotsInfo { total_slots, build_info })
}

/// Fetches and parses `{base_url}/props` in one call. `base_url` must
/// already be the bare origin (no `/v1` suffix) -- `/props`, like
/// `/slots`, is a native llama-server endpoint, not an OpenAI-compat one.
/// Fully fail-open: any failure (build, network, timeout, non-2xx,
/// malformed body) returns `None`, never propagates an error.
pub async fn fetch_llama_slots_info(base_url: &str) -> Option<LlamaSlotsInfo> {
    let client = reqwest::Client::builder()
        .timeout(KVCACHE_PROBE_TIMEOUT)
        .build()
        .ok()?;
    let url = format!("{}/props", base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    parse_llama_slots_info(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_llama_slots_info_from_real_props_shape() {
        // Real /props response shape, confirmed against a live
        // llama-server during aivyx-coder's own adoption (2026-08-21).
        let json = serde_json::json!({
            "default_generation_settings": {"params": {}},
            "total_slots": 4,
            "model_path": "/home/me/models/model.gguf",
            "build_info": "b10107-3121043"
        });
        let info = parse_llama_slots_info(&json).expect("must parse a real llama-server /props body");
        assert_eq!(info.total_slots, 4);
        assert_eq!(info.build_info, "b10107-3121043");
    }

    #[test]
    fn parse_llama_slots_info_returns_none_when_fields_are_absent() {
        let json = serde_json::json!({"some_other_server": true});
        assert!(parse_llama_slots_info(&json).is_none());
    }

    #[tokio::test]
    async fn fetch_llama_slots_info_returns_none_when_nothing_is_listening() {
        // No server at all on this port -- confirms the fail-open path
        // returns None rather than panicking or hanging past the timeout.
        let result = fetch_llama_slots_info("http://127.0.0.1:1").await;
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Wire the module in and run the tests**

Add to `crates/aivyx-llm/src/lib.rs`:

```rust
mod kvcache_probe;
pub use kvcache_probe::{KVCACHE_PROBE_TIMEOUT, LlamaSlotsInfo, fetch_llama_slots_info, parse_llama_slots_info};
```

Run: `cargo test -p aivyx-llm kvcache_probe -- --test-threads=1`
Expected: PASS, 3/3 new tests (the third genuinely exercises a real, immediate connection-refused failure — no mock server needed, `127.0.0.1:1` refuses instantly).

- [ ] **Step 3: Commit**

```bash
git add crates/aivyx-llm/src/kvcache_probe.rs crates/aivyx-llm/src/lib.rs
git commit -m "feat: add a /props probe for total_slots + build_info"
```

---

### Task 4: The checkout/warm-up/restore/pin/release policy in `LlmPlanner`

**Files:**
- Modify: `crates/aivyx-core/src/llm_planner.rs`
- Modify: `crates/aivyx-core/Cargo.toml` (add `aivyx-kvcache` dependency)

**Interfaces:**
- Consumes: `KvSlotPool` (Task 2), `LlmRequest.id_slot` (Task 1), `aivyx_kvcache::{CacheKey, CacheMeta, KvCacheStore, LlamaServerSlotStore}`.
- Produces: `LlmPlanner::with_kv_cache(mut self, pool: Arc<KvSlotPool>, store: Arc<LlamaServerSlotStore>, backend_id: String, model_id: String, build_hash: String) -> Self` — a chainable builder method matching this codebase's own established idiom (`ConcreteAgent::with_checkpointer`, `SpecialistFactory::with_checkpointer`, `OpenAiConfig::with_base_url` — all `with_X(mut self, ...) -> Self`, not `&mut self` setters). Absent this call, every code path this task adds is a complete no-op.

**Two real architectural facts this task's design depends on, both verified against real code, not assumed:**
1. `begin_turn` (`crates/aivyx-core/src/llm_planner.rs:723`) is the `TurnPlanner` trait's own per-turn entry point (`crates/aivyx-core/src/planner.rs:101-109`) — called exactly once per turn, before any `next_step`/`one_step` call. This is where checkout+warm-up belongs (the direct analog of `aivyx-coder`'s pre-loop `ensure_kv_slot_checked_out` call).
2. `LlmPlannerConfig.conversation_seeder: Option<Arc<dyn ConversationSeeder>>` (`crates/aivyx-core/src/llm_planner.rs:301-306`) means `begin_turn` can seed `self.history` with **real prior conversation** for channels with multi-turn continuity wired. The warm-up request must never read `self.history` at all — build it purely from `self.config.system_prompt` + `self.tools`, exactly as carefully as `aivyx-coder`'s own `system_prompt_text()` avoided that repo's own history field. Unlike `aivyx-coder`, no refactor is needed to get a history-free system prompt here — `LlmPlannerConfig.system_prompt: Option<String>` is already a clean, separate field (`crates/aivyx-core/src/llm_planner.rs:270-271`), never combined with history at the type level.

- [ ] **Step 1: Add the `aivyx-kvcache` dependency**

In `crates/aivyx-core/Cargo.toml`, add to `[dependencies]`:

```toml
aivyx-kvcache = { git = "https://github.com/Aivyx-Agent/aivyx-kvcache", rev = "e1b06c9960ee98841d9b91978a11dd99ed388490" }
```

Run: `cargo build -p aivyx-core` — expected: succeeds, fetching the new dependency.

- [ ] **Step 2: Write the failing tests for `compute_prefix_hash()`**

This is a pure function, easy to test without any real HTTP. Add to `crates/aivyx-core/src/llm_planner.rs`'s existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn compute_prefix_hash_is_stable_for_identical_inputs() {
        let tools = vec![LlmToolDescriptor {
            name: "read_file".to_string(),
            description: "reads a file".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let h1 = compute_prefix_hash(Some("system prompt text"), &tools);
        let h2 = compute_prefix_hash(Some("system prompt text"), &tools);
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_prefix_hash_differs_when_system_text_differs() {
        let tools: Vec<LlmToolDescriptor> = vec![];
        let h1 = compute_prefix_hash(Some("prompt A"), &tools);
        let h2 = compute_prefix_hash(Some("prompt B"), &tools);
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_prefix_hash_differs_when_tools_differ() {
        let system = Some("same system text");
        let tools_a = vec![LlmToolDescriptor {
            name: "read_file".to_string(),
            description: "reads".to_string(),
            input_schema: serde_json::json!({}),
        }];
        let tools_b = vec![LlmToolDescriptor {
            name: "write_file".to_string(),
            description: "writes".to_string(),
            input_schema: serde_json::json!({}),
        }];
        assert_ne!(compute_prefix_hash(system, &tools_a), compute_prefix_hash(system, &tools_b));
    }

    #[test]
    fn compute_prefix_hash_treats_none_system_distinctly_from_empty_string() {
        let tools: Vec<LlmToolDescriptor> = vec![];
        assert_ne!(compute_prefix_hash(None, &tools), compute_prefix_hash(Some(""), &tools));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p aivyx-core compute_prefix_hash -- --test-threads=1`
Expected: FAIL to compile — `compute_prefix_hash` doesn't exist yet.

- [ ] **Step 4: Add `compute_prefix_hash`**

Add as a free function near the bottom of `crates/aivyx-core/src/llm_planner.rs` (alongside other small free helpers already in this file):

```rust
/// A stable-within-one-process-run hash of the stable prefix (system
/// prompt + tool definitions) -- used as `CacheKey.prefix_hash`.
/// Deliberately NOT guaranteed stable across Rust versions/compilations:
/// a rebuild changing the hash algorithm just means old kvcache entries
/// silently miss instead of hit (fail-open, matching every other
/// kvcache operation), never a correctness problem. `None` and `Some("")`
/// hash differently (a leading discriminant byte precedes the content)
/// so a planner with no system prompt at all never collides with one
/// whose prompt happens to be the empty string.
fn compute_prefix_hash(system_prompt: Option<&str>, tools: &[LlmToolDescriptor]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match system_prompt {
        Some(s) => {
            true.hash(&mut hasher);
            s.hash(&mut hasher);
        }
        None => false.hash(&mut hasher),
    }
    for tool in tools {
        tool.name.hash(&mut hasher);
        tool.description.hash(&mut hasher);
        tool.input_schema.to_string().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p aivyx-core compute_prefix_hash -- --test-threads=1`
Expected: PASS, 4/4 new tests.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-core/Cargo.toml crates/aivyx-core/src/llm_planner.rs
git commit -m "feat: add compute_prefix_hash() for the kvcache stable prefix"
```

- [ ] **Step 7: Add the `KvCacheConfig` field, `with_kv_cache` builder, checkout/warm-up/restore logic, and `Drop`**

Add near the top of `crates/aivyx-core/src/llm_planner.rs` (with the other imports):

```rust
use aivyx_kvcache::{CacheKey, CacheMeta, KvCacheStore, LlamaServerSlotStore};
use aivyx_llm::KvSlotPool;
```

Add a new private struct and two new `LlmPlanner` fields (after the existing `task_message_index: Option<usize>,` field, `crates/aivyx-core/src/llm_planner.rs:494`):

```rust
struct KvCacheConfig {
    pool: Arc<KvSlotPool>,
    store: Arc<LlamaServerSlotStore>,
    backend_id: String,
    model_id: String,
    build_hash: String,
}
```

```rust
    /// `None` unless `with_kv_cache` was called (only ever true when
    /// `[agent] provider = "llama_cpp"`) -- every other code path this
    /// task adds is a complete no-op when this is `None`.
    kv_cache: Option<KvCacheConfig>,
    /// The slot id checked out from `kv_cache`'s pool, set as soon as
    /// `checkout()` succeeds in `begin_turn` (before any `.await` point),
    /// not only on a fully successful warm-up/restore -- so `Drop` can
    /// always release it even if the rest of `begin_turn`'s async work
    /// never completes (e.g. the turn future is dropped mid-warm-up).
    kv_slot_id: Option<u32>,
```

Add both fields to `LlmPlanner::new`'s constructor body (`crates/aivyx-core/src/llm_planner.rs:529-539`, alongside `task_message_index: None,`):

```rust
            kv_cache: None,
            kv_slot_id: None,
```

Add the builder method (near `LlmPlanner::new`, or alongside `history()`/`pruned_message_count()`):

```rust
    /// Opts this `LlmPlanner` into KV-cache persistence against a
    /// llama-server backend. Only ever called by whoever builds this
    /// planner's factory closure when `[agent] provider = "llama_cpp"`
    /// -- every other provider never calls this, and every code path
    /// this enables is a complete no-op otherwise.
    pub fn with_kv_cache(
        mut self,
        pool: Arc<KvSlotPool>,
        store: Arc<LlamaServerSlotStore>,
        backend_id: String,
        model_id: String,
        build_hash: String,
    ) -> Self {
        self.kv_cache = Some(KvCacheConfig { pool, store, backend_id, model_id, build_hash });
        self
    }
```

Add the checkout/warm-up/restore method:

```rust
    /// Checks out a slot and either restores a previously-saved matching
    /// prefix into it, or warms it fresh with exactly this turn's stable
    /// prefix (system prompt + tool defs -- never `self.history`, which
    /// may hold real prior conversation seeded by a `ConversationSeeder`)
    /// and saves it for future turns. Every failure mode past the
    /// pool-checkout itself is fail-open: logged at `warn`, the turn
    /// simply runs with an un-warmed (but still correctly pool-owned)
    /// slot. Called once, from `begin_turn`, before anything touches
    /// `self.history`.
    async fn ensure_kv_slot_checked_out(&mut self) {
        let Some(kv) = &self.kv_cache else {
            return; // kvcache not configured for this planner
        };
        let Some(slot_id) = kv.pool.checkout() else {
            tracing::warn!("kvcache: no free slot in the pool; this turn runs unpinned");
            return;
        };
        self.kv_slot_id = Some(slot_id);

        let key = CacheKey {
            backend_id: kv.backend_id.clone(),
            model_id: kv.model_id.clone(),
            build_hash: kv.build_hash.clone(),
            prefix_hash: compute_prefix_hash(self.config.system_prompt.as_deref(), &self.tools),
        };

        let restored = match kv.store.restore_into_slot(&key, slot_id).await {
            Ok(true) => true,
            Ok(false) => false,
            Err(err) => {
                tracing::warn!(error = %err, "kvcache: restore_into_slot failed");
                false
            }
        };

        if !restored {
            // Cold (or a stale/rejected restore -- Manifest::insert is an
            // upsert, so this cleanly repairs a stuck-cold key too):
            // warm the slot with exactly the stable prefix, save it, then
            // proceed. The warm-up goes through the *same* provider the
            // real turn uses (not a raw HTTP call) so its tokenization
            // matches exactly -- a mismatch here silently defeats
            // automatic reuse. The trailing empty User message is
            // required, not decorative: confirmed live against a real
            // llama-server (Qwen3.5's chat template) that a
            // system-message-only request is REJECTED outright (400, "No
            // user query found in messages").
            let warm_up_messages: Vec<LlmMessage> = vec![LlmMessage {
                role: LlmRole::User,
                content: vec![ContentBlock::text("")],
            }];
            let warm_up_request = LlmRequest {
                model: &kv.model_id,
                system: self.config.system_prompt.as_deref(),
                messages: &warm_up_messages,
                tools: &self.tools,
                max_tokens: 1,
                temperature: None,
                id_slot: Some(slot_id),
            };
            let cancellation = CancellationToken::new();
            match self.provider.chat_stream(warm_up_request, &cancellation).await {
                Ok(mut stream) => {
                    let mut warm_up_failed = false;
                    loop {
                        match stream.next_event().await {
                            Ok(Some(_)) => {}
                            Ok(None) => break,
                            Err(_) => {
                                warm_up_failed = true;
                                break;
                            }
                        }
                    }
                    if warm_up_failed {
                        tracing::warn!(
                            "kvcache: warm-up stream errored mid-response; skipping save so a \
                             partial/corrupt slot is never recorded as a valid cache entry"
                        );
                    } else {
                        let meta = CacheMeta { size_bytes: 1, token_count: 1 };
                        if let Err(err) = kv.store.save_from_slot(&key, slot_id, meta).await {
                            tracing::warn!(error = %err, "kvcache: save_from_slot failed");
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "kvcache: warm-up request failed");
                }
            }
        }
    }
```

(`LlmStream`'s exact draining shape -- `next_event()`/`finish()` vs. a plain `Stream` -- must match this file's own real trait; check `one_step`'s existing drain loop, a few lines below this insertion point in the same file, and mirror its exact method names/error handling rather than the sketch above if they differ. The sketch above assumes an event-by-event pull loop returning `Result<Option<_>, LlmError>>` per call, matching `LlmStream`'s doc comment quoted earlier in this same file — "the stream yields zero or more of these, then returns `None`, then the caller calls `finish`" — adapt precisely to whatever `one_step` actually calls.)

Call it from `begin_turn`, as the very first line, before any history mutation (`crates/aivyx-core/src/llm_planner.rs:723`):

```rust
    async fn begin_turn(&mut self, message: &Message, turn_id: crate::TurnId) {
        self.ensure_kv_slot_checked_out().await;
        let mut content = match &message.content {
```

Thread `id_slot` into the real `LlmRequest` built inside `one_step` (`crates/aivyx-core/src/llm_planner.rs:583-590`) — add one field to the existing struct literal:

```rust
        let request = LlmRequest {
            model: self.config.model.as_str(),
            system: self.config.system_prompt.as_deref(),
            messages: &self.history,
            tools: &self.tools,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            id_slot: self.kv_slot_id,
        };
```

Add `impl Drop for LlmPlanner` (this struct has none today — a new impl block, not a modification of an existing one):

```rust
impl Drop for LlmPlanner {
    /// Releases this turn's checked-out kvcache slot, if any. Pure,
    /// synchronous, infallible -- `KvSlotPool::release` does no I/O, so
    /// this is safe to run from `Drop` (which cannot be async). Fires
    /// naturally when this planner (a local variable inside
    /// `ConcreteAgent::turn()`, freshly constructed every turn) goes out
    /// of scope at the end of the turn -- no separate release call site
    /// needed anywhere.
    fn drop(&mut self) {
        if let (Some(slot_id), Some(kv)) = (self.kv_slot_id, &self.kv_cache) {
            kv.pool.release(slot_id);
        }
    }
}
```

- [ ] **Step 8: Run the full test suite**

Run: `cargo test -p aivyx-core -- --test-threads=1`
Expected: PASS. Every pre-existing `LlmPlanner` test constructs a planner without calling `with_kv_cache`, so `ensure_kv_slot_checked_out` returns immediately (`kv_cache` is `None`) and `LlmRequest.id_slot` stays `None` throughout — confirming this task adds no behavior change for any test (or config) that doesn't opt in.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-core/src/llm_planner.rs
git commit -m "feat: checkout/warm-up/restore/pin/release kvcache policy in LlmPlanner"
```

---

### Task 5: Wire the shared pool/store into the daemon's main agent

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`
- Modify: `crates/aivyx-cli/Cargo.toml` (add `aivyx-kvcache` dependency)

**Interfaces:**
- Consumes: `LlmPlanner::with_kv_cache` (Task 4), `KvSlotPool::new` (Task 2), `aivyx_llm::fetch_llama_slots_info` (Task 3).
- Produces: `kv_cache_handles: Option<(Arc<aivyx_llm::KvSlotPool>, Arc<aivyx_kvcache::LlamaServerSlotStore>, String)>` — a local binding in `run_async`, constructed once, right after the `ProviderKind::LlamaCpp` provider-construction arm. Task 6 threads the same binding into the two `aivyx-team` call sites in this same function and in the other two files that build teams.

- [ ] **Step 1: Build the shared pool + store, right after the provider-selection `match`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, the whole provider-selection block is `let provider: Arc<dyn LlmProvider> = match provider_kind.value { ... };` (`crates/aivyx-cli/src/bin/aivyx.rs:5771`); the `ProviderKind::LlamaCpp` arm (`5839-5865`) builds its own local `base_url` (`5852-5854`, already the bare origin — see this plan's own Global Constraints) and consumes it into `OpenAiConfig` before the arm ends, so it isn't in scope after the match. Capture a clone before that move, in a variable declared just before the match:

Change `crates/aivyx-cli/src/bin/aivyx.rs:5852-5854` from:

```rust
            let base_url = openai_base_url
                .map(|s| s.value)
                .unwrap_or_else(|| DEFAULT_LLAMACPP_BASE_URL.to_string());
```

to:

```rust
            let base_url = openai_base_url
                .map(|s| s.value)
                .unwrap_or_else(|| DEFAULT_LLAMACPP_BASE_URL.to_string());
            llamacpp_base_url_for_kvcache = Some(base_url.clone());
```

And add the declaration immediately before the match at line 5771:

```rust
    let mut llamacpp_base_url_for_kvcache: Option<String> = None;
    let provider: Arc<dyn LlmProvider> = match provider_kind.value {
```

Then, right after the whole match's closing `};` (i.e. right after `provider`'s own `let` statement completes — find where the next statement using `provider` begins and insert before it), add:

```rust
    let kv_cache_handles = match llamacpp_base_url_for_kvcache {
        Some(base_url) => match aivyx_llm::fetch_llama_slots_info(&base_url).await {
            Some(info) => {
                let store_path = directories::ProjectDirs::from("", "", "aivyx")
                    .map(|dirs| dirs.data_local_dir().join("kvcache"))
                    .unwrap_or_else(|| std::env::temp_dir().join("aivyx").join("kvcache"));
                match aivyx_kvcache::LlamaServerSlotStore::open(
                    &store_path,
                    &base_url,
                    10 * 1024 * 1024 * 1024, // 10 GiB default budget
                ) {
                    Ok(store) => Some((
                        Arc::new(aivyx_llm::KvSlotPool::new(info.total_slots)),
                        Arc::new(store),
                        info.build_info,
                    )),
                    Err(err) => {
                        tracing::warn!(error = %err, "kvcache: failed to open store; disabled for this run");
                        None
                    }
                }
            }
            None => {
                tracing::warn!(
                    "kvcache: [agent] provider = \"llama_cpp\" but /props probe failed or \
                     didn't look like a real llama-server response; disabled for this run"
                );
                None
            }
        },
        None => None, // not the LlamaCpp arm this run
    };
```

`directories` is not yet a dependency of `crates/aivyx-cli` — check `crates/aivyx-cli/Cargo.toml` and add it if missing (match whatever version `directories` is already pinned to elsewhere in this workspace's own XDG-path resolution, e.g. `crates/aivyx-config/Cargo.toml`).

- [ ] **Step 2: Call `with_kv_cache` on the daemon's main-agent planner factory closure**

The closure (`crates/aivyx-cli/src/bin/aivyx.rs:8697`, `let planner_factory = move || { ... };`) already clones `planner_config` into a per-turn `cfg` (`8698`, `let mut cfg = planner_config.clone();`) and ends by constructing `Box::new(LlmPlanner::new(Arc::clone(&planner_provider), Arc::clone(&planner_tools), cfg)) as Box<dyn aivyx_core::TurnPlanner>` (`8726-8730`). `cfg.model` already carries the real served model-id string (set when `planner_config` was first built, before this closure) — reuse it directly, no new binding needed.

Before the closure literal (`crates/aivyx-cli/src/bin/aivyx.rs:8697`, alongside where `planner_provider`/`planner_tools` are prepared for capture at `8682-8683`), add:

```rust
        let planner_kv_cache_handles = kv_cache_handles.clone();
```

Change the closure's own final expression (`8726-8730`) from:

```rust
            Box::new(LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_tools),
                cfg,
            )) as Box<dyn aivyx_core::TurnPlanner>
```

to:

```rust
            let planner = LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_tools),
                cfg.clone(),
            );
            let planner = match &planner_kv_cache_handles {
                Some((pool, store, build_hash)) => planner.with_kv_cache(
                    Arc::clone(pool),
                    Arc::clone(store),
                    "llama-server".to_string(),
                    cfg.model.clone(),
                    build_hash.clone(),
                ),
                None => planner,
            };
            Box::new(planner) as Box<dyn aivyx_core::TurnPlanner>
```

(`cfg.clone()` in the first line since `cfg.model` is read again below it — `LlmPlannerConfig` already derives `Clone`, confirmed at `crates/aivyx-core/src/llm_planner.rs:264`.)

- [ ] **Step 3: Add the dependency and build**

Add to `crates/aivyx-cli/Cargo.toml`:

```toml
aivyx-kvcache = { git = "https://github.com/Aivyx-Agent/aivyx-kvcache", rev = "e1b06c9960ee98841d9b91978a11dd99ed388490" }
```

Run: `cargo build --workspace`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs crates/aivyx-cli/Cargo.toml
git commit -m "feat: wire kvcache pool/store into the daemon's main-agent planner"
```

---

### Task 6: Wire the shared pool/store into `aivyx-team`'s specialists

**Files:**
- Modify: `crates/aivyx-team/src/factory.rs`
- Modify: `crates/aivyx-team/src/assembly.rs`
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`
- Modify: `crates/aivyx-team/Cargo.toml` (add `aivyx-kvcache` + `aivyx-llm` if not already present — check first, `aivyx-llm` is almost certainly already a dependency given `SpecialistFactory` already takes `Arc<dyn LlmProvider>`)

**Interfaces:**
- Consumes: `LlmPlanner::with_kv_cache` (Task 4), Task 5's `kv_cache_handles` binding (reused, not reconstructed — every specialist/lead turn shares the exact same pool/store instances the main daemon agent uses, sized to the one real `total_slots`).
- Produces: `SpecialistFactory::with_kv_cache(mut self, handles: Option<(Arc<aivyx_llm::KvSlotPool>, Arc<aivyx_kvcache::LlamaServerSlotStore>, String)>) -> Self`, matching `with_checkpointer`'s exact existing shape (`crates/aivyx-team/src/factory.rs:106-112`). `TeamAssembly::build` gains one new trailing parameter, `kv_cache_handles`, matching how `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>` is already its own trailing parameter (`crates/aivyx-team/src/assembly.rs:63`).

- [ ] **Step 1: Add `with_kv_cache` to `SpecialistFactory`**

In `crates/aivyx-team/src/factory.rs`, add a field to `SpecialistFactory` (after `checkpointer`, `crates/aivyx-team/src/factory.rs:59-62`):

```rust
    /// The shared kvcache pool/store + served build hash, when `[agent]
    /// provider = "llama_cpp"` -- attached to every built specialist so
    /// its own per-turn `LlmPlanner` shares the exact same `KvSlotPool`
    /// the daemon's main agent uses, not one each. `None` (the default)
    /// disables kvcache for every specialist this factory builds.
    kv_cache_handles: Option<(
        Arc<aivyx_llm::KvSlotPool>,
        Arc<aivyx_kvcache::LlamaServerSlotStore>,
        String,
    )>,
```

Initialize it in `SpecialistFactory::new` (alongside `checkpointer: None,`, `crates/aivyx-team/src/factory.rs:73-82`):

```rust
            kv_cache_handles: None,
```

Add the builder method (right after `with_checkpointer`, `crates/aivyx-team/src/factory.rs:106-112`, matching its exact shape):

```rust
    /// Attach the shared kvcache pool/store to every specialist this
    /// factory builds. `None` means "no kvcache" (provider isn't
    /// llama-server, or the `/props` probe failed), preserving
    /// pre-kvcache behavior -- same shape as `with_checkpointer`.
    pub fn with_kv_cache(
        mut self,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
    ) -> Self {
        self.kv_cache_handles = kv_cache_handles;
        self
    }
```

In `build`'s own closure (`crates/aivyx-team/src/factory.rs:143-152`), capture `self.kv_cache_handles.clone()` before the closure and chain `.with_kv_cache(...)` onto the constructed planner:

```rust
        let kv_cache_handles = self.kv_cache_handles.clone();
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            Arc::clone(&self.audit),
            move || {
                let cfg = LlmPlannerConfig::new(&model)
                    .with_system_prompt(&soul)
                    .with_max_tokens(max_tokens);
                let planner = LlmPlanner::new(
                    Arc::clone(&provider),
                    Arc::clone(&registry_for_planner),
                    cfg,
                );
                let planner = match &kv_cache_handles {
                    Some((pool, store, build_hash)) => planner.with_kv_cache(
                        Arc::clone(pool),
                        Arc::clone(store),
                        "llama-server".to_string(),
                        model.clone(),
                        build_hash.clone(),
                    ),
                    None => planner,
                };
                Box::new(planner)
            },
        )
        .with_checkpointer(self.checkpointer.clone());
```

- [ ] **Step 2: Thread it through `TeamAssembly::build`**

In `crates/aivyx-team/src/assembly.rs`, add a new trailing parameter to `build`'s signature (after `checkpointer`, `crates/aivyx-team/src/assembly.rs:63`):

```rust
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
    ) -> Result<Self, TeamError> {
```

And chain `.with_kv_cache(kv_cache_handles)` onto the factory construction (`crates/aivyx-team/src/assembly.rs:70-73`):

```rust
        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends)
            .with_checkpointer(checkpointer)
            .with_kv_cache(kv_cache_handles);
```

- [ ] **Step 3: Update both real call sites of `TeamAssembly::build`**

Both `crates/aivyx-channel/src/team_mission_driver.rs:987`+ and `crates/aivyx-cli/src/bin/aivyx_modules/team.rs:204`+ call `TeamAssembly::build(...)` with an explicit argument list — add `kv_cache_handles` as the new trailing argument at each.

For `crates/aivyx-cli/src/bin/aivyx_modules/team.rs` (the `aivyx team run` CLI path — `team::run_mission`, this same function received `checkpointer` as a parameter already), `run_mission`'s own signature needs `kv_cache_handles` threaded in as a new parameter too, and its caller (in `crates/aivyx-cli/src/bin/aivyx.rs`, wherever `team::run_mission(...)` is invoked — the CLI-mode dispatch, separate from the daemon's own `run_async`) needs to pass Task 5's `kv_cache_handles` binding through. This CLI path does **not** share the daemon's own long-lived process — `aivyx team run` builds its own provider from scratch per invocation, so it needs its **own** `/props` probe + pool/store construction (mirroring Task 5's Step 1 exactly, in whatever function builds the provider for this CLI path — check `crates/aivyx-cli/src/bin/aivyx.rs` near `team::run_mission`'s own call site for where the provider is built there).

For `crates/aivyx-channel/src/team_mission_driver.rs` (the daemon-hosted persistent-mission path), find its own caller inside `run_async` (the same function Task 5 modified) — this path runs inside the same long-lived daemon process, so it should reuse Task 5's **same** `kv_cache_handles` binding, not probe a second time.

- [ ] **Step 4: Update test call sites to compile**

The `SpecialistFactory::new(...)` test call sites (`crates/aivyx-team/src/factory.rs:266,457,543`, `crates/aivyx-team/src/pool.rs:442`, `crates/aivyx-team/src/testutil.rs:225`) don't need changes — `with_kv_cache` is a chainable builder with its own `None` default via `SpecialistFactory::new`'s own constructor, so uncalled tests keep compiling unchanged. `TeamAssembly::build(...)` test call sites (`crates/aivyx-team/src/assembly.rs:180,198,262`) DO need the new trailing argument added — add `None,` to each (no kvcache in these tests).

- [ ] **Step 5: Build and run the full existing test suite**

Run: `cargo build --workspace && cargo test --workspace -- --test-threads=1`
Expected: builds cleanly, all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-team/src/factory.rs crates/aivyx-team/src/assembly.rs crates/aivyx-channel/src/team_mission_driver.rs crates/aivyx-cli/src/bin/aivyx_modules/team.rs crates/aivyx-cli/src/bin/aivyx.rs crates/aivyx-team/Cargo.toml
git commit -m "feat: wire kvcache pool/store into aivyx-team specialists and leads"
```

---

### Task 7: Real end-to-end verification on the GPU rig

**Files:** none modified — this task verifies Tasks 1-6 against a real `llama-server` and produces a PASS/FAIL report. If it fails for a reason that points to a real bug in an earlier task's code, fix that task's file and re-run this task; don't mark it done on a failing run.

**Interfaces:**
- Consumes: everything from Tasks 1-6, plus the same GPU rig and Rust toolchain both prior `aivyx-kvcache`/`aivyx-coder` kvcache projects already used.

**Context for whoever runs this task:** this needs a real, running `llama-server` with a real GGUF model, matching every prior live-rig task this session. If you are a subagent without access to that real infrastructure, report `NEEDS_CONTEXT` rather than guessing. This task is best run under the controller's direct supervision, same as the analogous task in `aivyx-coder`'s own kvcache-adoption plan.

- [ ] **Step 1: Build the `aivyx` binary locally**

```bash
cargo build --release -p aivyx-cli
```

- [ ] **Step 2: Copy it to the rig and configure it**

```bash
scp target/release/aivyx 10.80.80.148:~/.local/bin/aivyx
ssh 10.80.80.148 'chmod +x ~/.local/bin/aivyx'
```

Set `[agent] provider = "llama_cpp"` in whatever `aivyx.toml` this run uses on the rig, pointed at the rig's own `llama-server` (reuse the exact rig conventions already established in the prior two kvcache plans: model at `/home/julian/models/Qwen3.5-9B-Q4_K_M.gguf`, `llama-server` needs `--slot-save-path` pointed at exactly wherever `directories::ProjectDirs::from("", "", "aivyx").data_local_dir().join("kvcache").join("slots")` resolves to for the account running this — confirm the real path by running the binary once with kvcache logging and reading what it actually opens, rather than assuming, matching the lesson from `aivyx-coder`'s own live verification where an ad hoc manual `--slot-save-path` didn't initially match the real client-side expectation).

- [ ] **Step 3: Run one turn, restart the server, run an equivalent turn again**

Use `aivyx team run "<mission>" --config <a minimal team pack>` (see `aivyx/docs/NONAGON.md` for the real `[[team.member]]` shape, and `aivyx/docs/superpowers/plans/2026-08-21-aivyx-coder-nonagon-specialist.md`'s own Task 3 for a worked example of exactly this kind of scratch live-verification team pack) or the daemon's own interactive path, whichever is faster to stand up on the rig — either exercises `LlmPlanner::begin_turn` for real.

Check the kvcache store actually has a saved entry after the first turn (`find`/`sqlite3` against the manifest, same technique the two prior kvcache verification tasks this session already used), restart `llama-server` for real (confirm via a genuine model-reload log line or a `/health` 503→200 transition, not just a process restart), then run an equivalent second turn on the same repo/config (same stable prefix) and confirm: no `kvcache: ... failed` warnings in the log, the manifest's single row shows the real measured file size (not the `1`-byte placeholder), and — if `AIVYX_DEBUG_LOG`-equivalent wire capture is available for `aivyx` (check whether one exists; if not, this step's proof is the absence of warnings plus the manifest's real recorded size) — `id_slot` appears in the real wire traffic on the second run.

- [ ] **Step 4: Record the result**

No commit for this task. Report PASS with what was actually observed, or FAIL with the exact failure and which earlier task's code it points to — for inclusion in `aivyx-ecosystem/ROADMAP.md`'s closing update for this whole combined kvcache-adoption initiative (all three plans, across three repos, complete once this one passes).

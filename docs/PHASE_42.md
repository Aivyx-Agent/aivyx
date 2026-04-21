# Phase 42 — Shell Hardening & Memory GC

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close the two most important operational safety gaps for a
daily-use daemon: shell child-process cleanup and unbounded
memory growth. Process-group execution prevents zombie
grandchildren from outliving the turn. Memory GC (per-topic
cap enforcement, TTL-based expiry, and an agent-invocable GC
tool) keeps the memory substrate bounded.

## Why now

1. **Zombie grandchildren.** `shell.exec` spawns `sh -c <cmd>`,
   which may fork its own children. `kill_on_drop(true)` only
   SIGKILLs the direct child (`sh`), leaving grandchildren
   running indefinitely. For a daemon that runs 24/7, orphaned
   processes accumulate. `process_group(0)` puts the entire
   process tree under one PGID so a single signal kills them all.

2. **Shell env leakage.** The tool currently inherits the
   daemon's full environment. An LLM-chosen command could read
   `ANTHROPIC_API_KEY`, `AIVYX_PASSPHRASE`, or any other secret
   in the env. Clearing the env and injecting only safe defaults
   is a defense-in-depth measure.

3. **Unbounded memory growth.** The `memory.write` tool has a
   per-topic cap (`DEFAULT_MAX_PER_TOPIC = 10_000`) but the only
   enforcement is "refuse new writes." There is no way to evict
   old entries proactively, and no TTL expiry. A long-running
   daemon accumulates stale entries forever.

## Entry baseline

- Tests: 814
- Clippy warnings: 0
- Deferral backlog: 0
- DESIGN.md streak: 1 phase (touched in Phase 41 for A7)
- PRODUCT.md streak: 5 phases (untouched since Phase 38)
- lib.rs streak: 2 phases (untouched since Phase 40)

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | **touched** | Amendment for shell env + memory GC |
| PRODUCT.md | untouched (6) | Internal hardening, no product-shape |
| lib.rs | **at risk** | Mitigable by keeping memory.gc in channel layer |

## Product commitment coverage

- **G2 (Code Interaction):** Process-group shell execution
  hardens the shell tool for production use.
- **G3 (Memory Reflection):** Memory GC provides the substrate
  for bounded, healthy memory.
- **G6 (Bounded Storage):** Per-topic caps and TTL expiry
  prevent unbounded growth.

## Tasks

### Task 1 — Open commit + PHASE_42.md scaffold

This file.

### Task 2 — Process-group shell execution

Call `command.process_group(0)` so `sh -c` and all its children
share a PGID. On timeout, SIGTERM the process group, wait 2
seconds for graceful exit, then SIGKILL. Replaces the current
`kill_on_drop(true)` + tokio timeout drop pattern.

**Files:** `crates/aivyx-core/src/tools/shell.rs`,
`crates/aivyx-core/Cargo.toml` (add `libc` dep)

### Task 3 — Shell environment variable support

Add optional `env` field to `shell.exec` input schema. Clear
inherited env (`env_clear()`), inject only declared vars + safe
defaults (`PATH`, `HOME`, `USER`, `LANG`, `TERM`). No new
capability scope — env control is orthogonal to the cwd sandbox.

**Files:** `crates/aivyx-core/src/tools/shell.rs`

### Task 4 — Memory GC: per-topic cap enforcement

Extend `Memory` trait with `gc_topic(topic, max_entries) ->
Result<usize, MemoryError>`. Evict oldest entries when count
exceeds the cap. Implement in both `InMemoryMemory` and
`RedbMemory`.

**Files:** `crates/aivyx-memory/src/lib.rs`,
`crates/aivyx-memory/src/redb.rs`

### Task 5 — Memory GC: TTL-based expiry

Add `memory_ttl_secs` config field (default: `None`). Add
`gc_expired(now_secs) -> Result<usize, MemoryError>` to the
`Memory` trait. Daemon calls it on a 1-hour timer.

**Files:** `crates/aivyx-memory/src/lib.rs`,
`crates/aivyx-memory/src/redb.rs`,
`crates/aivyx-config/src/lib.rs`,
`crates/aivyx-channel/src/daemon_server.rs`

### Task 6 — `memory.gc` infrastructure tool

Agent-invocable GC trigger. Input: `{ topic, max_entries }`.
Scoped under `memory.gc` capability base (new base in
`aivyx-capability`). Registered in the channel layer only
(preserves lib.rs streak if possible).

**Files:** `crates/aivyx-channel/src/memory_gc_tool.rs` (new),
`crates/aivyx-capability/src/lib.rs`,
`crates/aivyx-channel/src/lib.rs`

### Task 7 — Exit freeze

Tests, streak report, `docs/ROADMAP.md` rollover,
`docs/README.md` phase table update, ship records and
exit criteria.

## Exit criteria

- [ ] `process_group(0)` on all shell.exec spawns; SIGTERM →
      wait → SIGKILL on timeout.
- [ ] Shell env cleared, only safe defaults + declared vars.
- [ ] `gc_topic` on Memory trait, implemented in both backends.
- [ ] `gc_expired` on Memory trait with TTL config field.
- [ ] Daemon 1-hour GC timer wired.
- [ ] `memory.gc` tool registered in channel layer.
- [ ] All tests pass with net-positive delta.
- [ ] Zero clippy warnings.
- [ ] DESIGN.md amendment filed (if scope warrants).
- [ ] PRODUCT.md untouched (streak -> 6).

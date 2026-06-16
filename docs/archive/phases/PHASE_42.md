# Phase 42 — Shell Hardening & Memory GC

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

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

## Task 1 — shipped (2026-04-21)

Phase open commit. Scaffolded `docs/PHASE_42.md`. Added
`libc = "0.2"` to workspace deps, `libc = { workspace = true }`
to `aivyx-core/Cargo.toml`. Commit `8da61bd`.

## Task 2 — shipped (2026-04-21)

Process-group shell execution. `command.process_group(0)` puts
`sh -c` and all children under a shared PGID. On timeout:
`libc::killpg(pgid, SIGTERM)`, background reaper task sleeps
2s then `killpg(pgid, SIGKILL)`. `kill_on_drop(true)` retained
as belt-and-suspenders. Switched from `command.output()` to
`command.spawn()` + `wait_with_output()` with explicit
`stdout(Stdio::piped())` and `stderr(Stdio::piped())`. PID
captured before `wait_with_output()` consumes `Child`. 4 new
tests. Commit `5f9263d`.

## Task 3 — shipped (2026-04-21)

Shell env isolation. `env_clear()` strips all inherited env,
then `SAFE_ENV_DEFAULTS` (`PATH`, `HOME`, `USER`, `LANG`,
`TERM`) are injected from the daemon's env, then any declared
`env` field entries. Input schema updated with `env` object
field. `input_env()` helper parses the map. 2 new tests
(env cleared, declared vars passed, safe defaults injected
covered by the 4 in Task 2). Commit `4ee8fcb`.

## Task 4 — shipped (2026-04-21)

Memory GC trait methods. Extended `Memory` trait with
`gc_topic(topic, max_entries) -> Result<usize, MemoryError>`
and `gc_expired(cutoff_secs) -> Result<usize, MemoryError>`.
Implemented in both `InMemoryMemory` (BTreeMap drain/retain)
and `RedbMemory` (prefix scan + delete). 13 new tests (8
in-memory, 5 on-disk). Commit `6ddc68f`.

## Task 5 — shipped (2026-04-21)

TTL-based expiry with daemon GC timer. Added
`memory_ttl_secs: Option<Sourced<u64>>` to `AivyxConfig`,
`ttl_secs: Option<u64>` to TOML `[memory]`, env
`AIVYX_MEMORY_TTL_SECS`. Extended `DaemonConfig` with
`memory: Option<Arc<dyn Memory>>` and `memory_ttl_secs:
Option<u64>`. Daemon spawns 1-hour `tokio::time::interval`
background task calling `gc_expired()` when TTL configured.
Respects shutdown `CancellationToken`. Banner prints TTL
when set. Commit `07f1004`.

## Task 6 — shipped (2026-04-21)

`memory.gc` infrastructure tool. New `MemoryGcTool` in
`crates/aivyx-channel/src/memory_gc_tool.rs`. Input:
`{ topic, max_entries }`. Returns `{ evicted: N }`.
`memory.gc` scope base added to `KNOWN_BASES` (34→35) and
`CEILING_TRUSTED`. Registered in binary tool list and
backcompat floor. 8 new tests. Commit `7eac5e4`.

## Exit criteria (final)

- [x] `process_group(0)` on all shell.exec spawns; SIGTERM →
      wait → SIGKILL on timeout.
- [x] Shell env cleared, only safe defaults + declared vars.
- [x] `gc_topic` on Memory trait, implemented in both backends.
- [x] `gc_expired` on Memory trait with TTL config field.
- [x] Daemon 1-hour GC timer wired.
- [x] `memory.gc` tool registered in channel layer.
- [x] All tests pass: 839 (entry: 814, delta: +25).
- [x] Zero clippy warnings.
- [x] DESIGN.md untouched this phase (streak 1 from Phase 41).
- [x] PRODUCT.md untouched (streak → 6).
- [x] lib.rs untouched (streak → 3 from Phase 40).

## Streak report

| Streak | Status | Count |
|---|---|---|
| DESIGN.md | Held (1 from Phase 41 A7) | 1 |
| PRODUCT.md | Held (untouched since Phase 38) | 6 |
| lib.rs | Held (untouched since Phase 40) | 3 |

## Decisions made during Phase 42 not in DESIGN.md

1. **SIGTERM-first process-group kill.** On timeout, the shell
   tool sends SIGTERM to the process group first, waits 2s,
   then SIGKILLs. This gives well-behaved children a chance to
   flush buffers. The reaper is a detached `tokio::spawn` task
   so it doesn't block the turn loop.

2. **No `get_recent` TTL filtering.** The plan mentioned filtering
   expired entries in `get_recent`, but this would couple the
   substrate to config. The hourly GC timer physically deletes
   expired entries, so `get_recent` naturally won't return them
   after the next cycle. For a continuously-running daemon this
   is sufficient.

3. **`memory.gc` in channel layer, not memory crate.** Preserves
   the lib.rs streak. The memory crate provides the substrate
   methods; the channel crate wraps them as an agent-facing Tool.
   Same pattern as `OllamaListTool` and `ReflectionTool`.

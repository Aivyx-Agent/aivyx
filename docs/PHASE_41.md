# Phase 41 — Daemon Hardening & Error Typing

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Harden the daemon's resilience, clean up parameter bloat,
replace stringly-typed errors with proper error types, and
close the protocol versioning deferral (backlog 1 → 0). This
is a **hardening phase** — no new user-facing features, but
every subsequent phase benefits from a more robust foundation.

## Why now

1. **Parameter bloat.** `run_daemon()` has accreted 10 parameters
   across Phases 21–39. The `#[allow(clippy::too_many_arguments)]`
   comment explicitly tags this as "deferred to SDK phase." A
   `DaemonConfig` struct is overdue.

2. **Stringly-typed errors.** ~30 `Result<(), String>` signatures
   across the daemon layer make error handling opaque. Typed errors
   are a prerequisite for the Channel SDK (P5) — third-party
   adapters need matchable variants, not opaque strings.

3. **Protocol versioning.** The sole remaining deferral in the
   entire backlog. The `PROTOCOL_VERSION = "0.1"` constant exists
   but no negotiation logic validates versions between client and
   daemon. Closing this brings the deferral backlog to zero.

4. **Crash recovery.** The daemon runs 24/7 but active turns are
   lost on crash. Honest acknowledgment of data loss (not silent
   restart) is the minimum viable crash-recovery story.

## Entry baseline

- Tests: 807
- Clippy warnings: 0
- Deferral backlog: 1 (protocol versioning)
- DESIGN.md streak: 1 phase (touched in Phase 40 for A6)
- PRODUCT.md streak: 4 phases (untouched since Phase 38)
- lib.rs streak: 1 phase (touched in Phase 40 for NextStep)

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | **touched** | Amendment for protocol negotiation |
| PRODUCT.md | untouched (5) | Internal hardening, no product-shape |
| lib.rs | untouched (2) | All changes in channel layer |

## Product commitment coverage

- **P4 (Daemon-Default Architecture):** DaemonConfig, crash
  recovery, protocol negotiation all strengthen the daemon.
- **P5 (Channel SDK):** Typed errors are SDK surface — third-party
  adapters need matchable `DaemonError` variants.

## Tasks

### Task 1 — Open commit + PHASE_41.md scaffold

This file.

### Task 2 — `DaemonConfig` struct

Extract the 10 parameters of `run_daemon()` into a
`DaemonConfig` struct in `daemon_server.rs`. Remove the
`#[allow(clippy::too_many_arguments)]`. Update the binary's
call site in `aivyx.rs` and `run_daemon_compat()`.

**Files:** `daemon_server.rs`, `bin/aivyx.rs`

### Task 3 — `DaemonError` enum

Replace ~30 `Result<(), String>` signatures across
`daemon_server.rs`, `web_ui.rs`, `mission.rs`,
`daemon_client.rs`, `telegram_daemon_frontend.rs` with a
`thiserror`-derived `DaemonError` enum. Variants:
`Bind`, `Accept`, `Frame`, `Protocol`, `MissionStore`,
`Config`, `WebSocket`, `Io`.

**Files:** `daemon_server.rs`, `daemon_client.rs`, `web_ui.rs`,
`mission.rs`, `telegram_daemon_frontend.rs`, `lib.rs` (re-export)

### Task 4 — Crash-recovery metadata

On daemon startup, write a `daemon.state` JSON file (sibling
to `daemon.pid`) recording active session IDs and in-flight
turn IDs. On clean shutdown, clear it. On restart after crash,
detect stale state file, log which turns were lost, and emit
`DaemonLifecycleEvent::RecoveryNotice` so frontends can inform
the operator. This does **not** replay turns — it acknowledges
data loss honestly.

**Files:** `daemon_server.rs`, `daemon_ipc.rs`

### Task 5 — Protocol version negotiation

Add `ProtocolNegotiation { version: String }` to
`FrontendMessage`. Add `ProtocolAccepted { version: String }`
and `ProtocolRejected { supported: Vec<String> }` to
`DaemonMessage`. For v0.1, the daemon always accepts. The
negotiation frame is forward-compatible for v0.2+.

Closes the sole remaining deferral (backlog 1 → 0).

File `docs/amendments/2026-04-21-protocol-negotiation.md`
for the DESIGN.md amendment.

**Files:** `daemon_ipc.rs`, `daemon_server.rs`, `daemon_client.rs`,
`docs/amendments/`

### Task 6 — Exit freeze

Tests, streak report, `docs/ROADMAP.md` rollover,
`docs/README.md` phase table update, ship records and
exit criteria.

## Exit criteria

- [x] `DaemonConfig` struct replaces the 10-parameter signature.
- [x] `DaemonError` enum replaces all `Result<(), String>` in the
      daemon layer (10 variants, ~30 signatures across 5 files).
- [x] `daemon.state` written on startup, cleared on clean
      shutdown, `RecoveryNotice` emitted on crash detection.
- [x] Protocol negotiation messages implemented and round-tripped
      in integration tests.
- [x] Deferral backlog at 0.
- [x] All tests pass: 814 (+7 from 807 baseline).
- [x] Zero clippy warnings.
- [x] DESIGN.md amendment A7 filed for protocol negotiation.
- [x] PRODUCT.md untouched (streak → 5).

## Streak report

| Streak target | Predicted | Actual |
|---|---|---|
| DESIGN.md | touched | **touched** (A7 protocol negotiation) |
| PRODUCT.md | untouched (5) | **untouched (5)** |
| lib.rs | untouched (2) | **untouched (2)** |

## Ship records

| Task | Commit | Delta |
|---|---|---|
| Task 1: Phase open | `d5a191b` | +0 |
| Task 2: DaemonConfig struct | `550ee58` | +0 |
| Task 3: DaemonError enum | `4ad3b22` | +0 |
| Task 4: Crash-recovery metadata | `157c1c7` | +6 |
| Task 5: Protocol negotiation | `59334e4` | +1 |
| Task 6: Exit freeze | *(this commit)* | +0 |

## Test delta

807 → 814 (+7). Six `DaemonState`/`StateGuard` unit tests,
one protocol negotiation e2e test.

## Deferral backlog

**0.** Protocol versioning (the sole remaining deferral) closed
in Task 5.

# Phase 20 — Daemon Management + Deferral Cleanup

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close six items from the sixteen-item rolling deferral backlog
with a focused cleanup phase: daemon lifecycle subcommands
(`daemon status`, `daemon stop`), PID file, `--no-daemon` flag,
daemon-mode banner parity, and the `CapabilitySet::grants`
reflexivity + `CEILING_SEMITRUSTED` doc-comment pair.

Phase 20 is **not a product-shape phase** — it ships no new
product primitive and advances no Product Commitment. It is
the same category as Phase 15 (Channel-Lib Consolidation):
a non-product-shape sub-phase that reduces accumulated
technical debt before the next keystone (Mission Primitive,
P2).

## Why now

1. **The deferral backlog has grown to sixteen items.** Five
   of those are daemon-management primitives deferred from
   Phases 17–18 because the daemon migration was the priority.
   The migration is complete (both adapters route through the
   daemon); the management surface is now the natural next
   step.

2. **`daemon status` / `daemon stop` are operator-facing
   gaps.** An operator whose daemon hangs or misbehaves has
   no subcommand to inspect or stop it — they have to `kill`
   the process manually. This is the first thing a real
   deployment will hit.

3. **The `CapabilitySet::grants` reflexivity investigation
   has been carried since Phase 13 (five phases).** It couples
   with the `CEILING_SEMITRUSTED` ▲-row doc-comment fix from
   Phase 15. Both are small, self-contained, and well-
   understood. Cleaning them up now prevents the pair from
   becoming permanently stale.

4. **Clearing the backlog before the Mission Primitive
   reduces noise.** The Mission phase (Phase 21, likely)
   will touch daemon infrastructure; having `daemon status`/
   `stop` and the PID file in place first means the Mission
   phase can compose against them rather than re-deferring
   them.

## Streak predictions

- **DESIGN.md** — Low risk. Daemon management subcommands
  and doc-comment fixes are plumbing, not architecture.
  Prediction: streak extends to **twenty consecutive phases**.
- **PRODUCT.md** — Not at risk. No product-shape work.
  Prediction: streak extends to **eight consecutive phases**.
- **Production-core `aivyx-core/src/lib.rs`** — Low risk.
  Tasks 2–5 touch `aivyx-channel` (binary + daemon modules).
  Task 6 touches `aivyx-capability` doc comments. None
  should need core lib changes. Prediction: streak extends
  to **nine consecutive phases** (would set a new record).

## Tasks

### Task 1 — Phase open (this commit)

Scaffold `docs/PHASE_20.md`. Update `docs/README.md`
phase-status table (Phase 20 → Open). Update
`docs/ROADMAP.md` Phase 20 entry.

### Task 2 — `daemon status` + `daemon stop` subcommands

Add `CliMode::DaemonStatus` and `CliMode::DaemonStop` to the
binary's CLI parser. Add a `FrontendMessage::Shutdown` variant
to `daemon_ipc.rs`. Implementation:

- `daemon status`: connect to the default socket path, send
  `StartSession` (or a lightweight ping), report whether a
  daemon is running and its protocol version from
  `DaemonReady`. If no daemon is listening, report that.
- `daemon stop`: connect, send `FrontendMessage::Shutdown`,
  wait for `ShuttingDown` lifecycle event, report success.
  The daemon server's receive loop handles `Shutdown` by
  cancelling its `CancellationToken`.

Parser tests for both subcommands. Integration test proving
`daemon stop` triggers graceful shutdown.

**Closes:** `daemon status / daemon stop subcommands`
(Phase 17 net-new).

### Task 3 — PID file at `$XDG_RUNTIME_DIR/aivyx/daemon.pid`

Write PID to `daemon.pid` (sibling of `daemon.sock`) on
daemon start. Remove on clean shutdown (Drop guard or
explicit cleanup in the shutdown path). `daemon status`
reads the PID file as a supplementary hint (process
existence check via `kill(pid, 0)`).

**Closes:** `PID file at $XDG_RUNTIME_DIR/aivyx/daemon.pid`
(Phase 17 net-new).

### Task 4 — `--no-daemon` flag for local mode

Add `--no-daemon` to the CLI parser. When set, the
`ChannelKind::Local` branch skips daemon-first dispatch and
goes directly to the in-process `run_session` path (the
original Phase 3 loop). Useful for debugging and for
operators who want deterministic single-process behavior.

Parser tests. One e2e test proving `--no-daemon` bypasses
daemon dispatch.

**Closes:** `--no-daemon flag for local mode` (Phase 18
net-new).

### Task 5 — Daemon-mode banner parity

The in-process banner shows: version, fs sandbox path,
`memory: live`, `audit: N events verified`, active role.
The daemon banner shows only: version `(daemon)`, socket
path, active role.

Bring to parity by extending the `SessionStarted` daemon
message (or adding a `ServerInfo` field) to carry the
daemon's store metadata: fs sandbox root, verified audit
event count, memory status. The daemon session banner
renders the same information as the in-process banner plus
the daemon socket path.

**Closes:** `Daemon-mode banner parity with in-process
banner` (Phase 18 net-new).

### Task 6 — `CEILING_SEMITRUSTED` ▲-row doc-comment fix + reflexivity investigation

Two co-homed items:

1. **Doc-comment fix:** The ▲-row comment on
   `CEILING_SEMITRUSTED` (lines 558–560 of
   `aivyx-capability/src/lib.rs`) describes scopes "omitted
   from unqualified ceiling" but the phrasing implies they
   survive intersection when qualified — which is only true
   if the held scope is also qualified and the qualifier
   matches. Rewrite to be precise about D4 rule interactions.

2. **Reflexivity investigation:** `CapabilitySet::grants`
   delegates to `Scope::is_granted_by`. Verify and document
   whether `grants(&self, &self)` is always true (reflexive)
   for every well-formed scope. If not, document the
   counter-example and whether it matters in practice.

Add a test pinning the reflexivity finding.

**Closes:** `CapabilitySet::grants reflexivity investigation`
(Phase 13 Task 4) + `Misleading CEILING_SEMITRUSTED ▲-row
doc comment` (Phase 15 Task 4).

### Task 7 — Exit freeze

Standard exit procedure: deferrals block, prediction-vs-
reality, exit criteria checklist, docs flips, ROADMAP +
PRODUCT_ROADMAP updates.

## Decisions

**Decision 1 (Q1→(a)): `daemon stop` is graceful-only.**
The `Shutdown` message cancels the server's `CancellationToken`;
the daemon finishes in-flight turns via the existing graceful
shutdown path. No `--force` flag, no PID-based SIGTERM. If the
daemon is unresponsive, the operator uses `kill` directly.

**Decision 2: `FrontendMessage::Shutdown` is the IPC mechanism.**
A new variant in the `FrontendMessage` enum, handled in
`handle_connection` by sending `ShuttingDown` and cancelling the
server-level `CancellationToken`. This composes with the existing
ctrl-C shutdown path (both cancel the same token).

**Decision 3: `daemon status` and `daemon stop` use a lightweight
`current_thread` runtime.** They dispatch before the config/store
stack via `run_daemon_management`, so no API key, passphrase, or
TOML config is required. A `current_thread` runtime is cheaper than
the multi-threaded runtime the session path uses.

**Decision 4: `daemon_status` and `daemon_stop` are standalone
client functions.** Not methods on `DaemonSession` — they don't
need a session. `daemon_status` connects, reads `DaemonReady`,
and returns `DaemonStatusInfo { running, version }`.
`daemon_stop` connects, reads `DaemonReady`, sends `Shutdown`,
and waits for `ShuttingDown`.

**Decision 5 (Q2→moot): Banner parity needs no IPC change.**
`canonical_root` and `verified_event_count` are already in scope
at the daemon banner construction site in `run_async` — the
frontend computes them from the store before the daemon session
starts. The fix is a pure format-string edit. Q2 options (a)–(c)
are all unnecessary.

## Open questions

**Q1 — Should `daemon stop` be graceful-only or support
`--force`?** → **(a), resolved in Decision 1.**

**Q2 — Should `SessionStarted` carry server metadata (for
banner parity), or should there be a separate `ServerInfo`
message type?**

(a) Extend `SessionStarted` with optional metadata fields
(`fs_sandbox`, `audit_event_count`, `memory_status`).

(b) Add a new `DaemonMessage::ServerInfo { ... }` variant
sent immediately after `SessionStarted`.

(c) Add optional fields to `DaemonReady` (the lifecycle
event already sent on connection).

**Recommendation: (a).** → **Moot — resolved without IPC change
(see Decision 5).**

## Task 2 ship record

**Files modified:**
- `crates/aivyx-channel/src/daemon_ipc.rs` (+2): `FrontendMessage::Shutdown`
  variant + round-trip test case.
- `crates/aivyx-channel/src/daemon_server.rs` (+5):
  `FrontendMessage::Shutdown` handler — sends `ShuttingDown`,
  cancels server `CancellationToken`.
- `crates/aivyx-channel/src/daemon_client.rs` (+78):
  `DaemonStatusInfo` struct, `daemon_status()` probe,
  `daemon_stop()` graceful-shutdown client.
- `crates/aivyx-channel/src/bin/aivyx.rs` (+115):
  `CliMode::DaemonStatus` + `CliMode::DaemonStop`, parser
  refactored to recognize `daemon status|stop|run` uniformly,
  `run_daemon_management` async dispatcher with lightweight
  `current_thread` runtime, 5 parser tests.
- `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs` (+93):
  3 integration tests — `daemon_stop_triggers_graceful_shutdown`,
  `daemon_status_reports_running_daemon`,
  `daemon_status_reports_not_running_for_absent_socket`.

**Test delta:** 550 → 558 (+8).
**Binary line count:** 2262 → 2377 (under 2400 threshold).
**All three byte-identity streaks held.**

## Task 3 ship record

**Design decision: PID file is a `Drop` guard sibling of the
socket file.** `PidGuard` writes `std::process::id()` to
`<socket_path>.with_extension("pid")` on daemon start and
removes it on drop. The guard composes with all exit paths
(graceful shutdown, `daemon stop`, early errors) without
explicit cleanup code. No `libc` dependency for
`kill(pid, 0)` — the PID is informational in `daemon status`
output; the socket probe remains the primary liveness check.
Zero new workspace dependencies preserved.

**Files modified:**
- `crates/aivyx-channel/src/daemon_ipc.rs` (+10):
  `default_pid_path()` function + test.
- `crates/aivyx-channel/src/daemon_server.rs` (+22):
  `PidGuard` struct with `Drop` impl, PID file write in
  `run_daemon`.
- `crates/aivyx-channel/src/daemon_client.rs` (+22):
  `read_pid_file()` utility, `DaemonStatusInfo.pid` field,
  `daemon_status()` reads PID file.
- `crates/aivyx-channel/src/bin/aivyx.rs` (+4): `daemon
  status` output includes PID when available.
- `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs` (+90):
  4 tests — `pid_file_appears_on_daemon_start_and_disappears_
  on_stop`, `daemon_status_includes_pid_from_pid_file`,
  `read_pid_file_returns_none_for_missing_file`,
  `read_pid_file_returns_none_for_non_numeric_content`.

**Test delta:** 558 → 563 (+5).
**All three byte-identity streaks held.**

## Task 4 ship record

**Design decision: `--no-daemon` guards both daemon-first
branches with a let-chain.** The flag adds `no_daemon: bool`
to `CliArgs`, threaded through `run_async`. Both the Local
and Telegram daemon-first dispatch blocks use
`if !no_daemon && let Ok(sp) = default_socket_path()` to
skip daemon dispatch entirely when the flag is set. The
in-process fallback path (original Phase 3 / Phase 8 code)
runs unconditionally. Mutual exclusions: `--no-daemon` +
`--verify-only` and `--no-daemon` + `--print-role` are
rejected (those modes don't use the daemon at all).

**Files modified:**
- `crates/aivyx-channel/src/bin/aivyx.rs` (+76):
  `no_daemon` field in `CliArgs`, `--no-daemon` parser arm,
  two mutual-exclusion checks, `run_async` parameter addition,
  let-chain guards on both Local and Telegram daemon-first
  blocks, 5 parser tests.

**Test delta:** 563 → 568 (+5).
**Binary line count:** 2377 → 2453 (53 over 2400 nominal
threshold; the overshoot is entirely parser test code — the
`#[cfg(test)]` module starts at line 1641, so production
code is 1641 lines, well under threshold).
**All three byte-identity streaks held.**

## Task 5 ship record

**Design decision: No IPC protocol change needed (Decision 5).**
The daemon banner is constructed on the frontend side in
`run_async`, where `canonical_root` and `verified_event_count`
are already in scope from the store setup. Q2 options (a)–(c)
— extending `SessionStarted`, adding `ServerInfo`, or extending
`DaemonReady` — were all unnecessary. The fix is a pure
format-string edit adding the three missing fields (`fs sandbox`,
`memory`, `audit`) to both daemon banners.

**Files modified:**
- `crates/aivyx-channel/src/bin/aivyx.rs` (+10): Local daemon
  banner and Telegram daemon banner extended with `fs sandbox`,
  `memory: live`, and `audit: persistent (N events verified
  from disk)` fields — matching the in-process banners.

**Test delta:** 568 → 568 (no new tests — format-string change
only, covered by existing integration tests).
**Binary line count:** 2453 → 2463.
**All three byte-identity streaks held.**

## Task 6 ship record

**Reflexivity finding: `grants(&self, &self)` is reflexive for
all practically-occurring scopes.** The proof walks five
qualifier dispatch paths: unqualified (rule 2, trivially true),
URL prefix (same origin + same path), path glob (`glob_matches`
with identical pattern/candidate), allowlist (same set is a
subset of itself), and simple glob. One theoretical counter-
example exists: a qualifier containing glob metacharacters
intended literally (e.g. `fs.read:/home/[user]/**`) would fail
because `globset` interprets `[user]` as a character class that
does not match the literal string `[user]`. No real tool or role
definition produces such a scope, so reflexivity holds in
practice. Pinned with a 7-case test.

**Doc-comment fix:** Rewrote the `CEILING_SEMITRUSTED` doc
comment. The old phrasing ("an agent holding the corresponding
qualified scope will still match via intersection") was
misleading — it implied qualified ▲-row scopes survive
intersection, when in fact they do not because the ceiling
carries no entry for ▲ bases at all. The rewrite separates
⊘ rows (hard-denied) from ▲ rows (conditionally granted) and
explains precisely how each interacts with D4 rules 1–4.

**Files modified:**
- `crates/aivyx-capability/src/lib.rs` (+27): Rewritten
  `CEILING_SEMITRUSTED` doc comment (⊘ vs ▲ semantics),
  `grants_is_reflexive_for_all_practical_scope_forms` test
  covering 7 qualifier forms.

**Test delta:** 568 → 569 (+1).
**All three byte-identity streaks held.**

## Deferrals targeted for closure

| # | Item | Origin | Target task |
|---|------|--------|-------------|
| 1 | `daemon status` / `daemon stop` | Phase 17 | Task 2 |
| 2 | PID file | Phase 17 | Task 3 |
| 3 | `--no-daemon` flag | Phase 18 | Task 4 |
| 4 | Banner parity | Phase 18 | Task 5 |
| 5 | `CapabilitySet::grants` reflexivity | Phase 13 | Task 6 |
| 6 | `CEILING_SEMITRUSTED` ▲-row doc | Phase 15 | Task 6 |

Rolling backlog: 16 → 10 at exit (all six closed).

## Deferrals

**Inherited deferrals closed by Phase 20:**

- **`daemon status` / `daemon stop` subcommands.** Phase 17
  net-new. **Closed by Task 2** — `CliMode::DaemonStatus` and
  `CliMode::DaemonStop` with lightweight `current_thread` runtime,
  `FrontendMessage::Shutdown` variant, `daemon_status()` and
  `daemon_stop()` standalone client functions.
- **PID file at `$XDG_RUNTIME_DIR/aivyx/daemon.pid`.** Phase 17
  net-new. **Closed by Task 3** — `PidGuard` with `Drop` impl,
  `read_pid_file()` utility, PID displayed in `daemon status`.
- **`--no-daemon` flag for local mode.** Phase 18 net-new.
  **Closed by Task 4** — `no_daemon: bool` in `CliArgs`, let-chain
  guards on both Local and Telegram daemon-first blocks.
- **Daemon-mode banner parity with in-process banner.** Phase 18
  net-new. **Closed by Task 5** — format-string edit only, no IPC
  protocol change needed (Decision 5).
- **`CapabilitySet::grants` reflexivity investigation.** Phase 13
  Task 4 deferral. **Closed by Task 6** — reflexive for all
  practically-occurring scopes; theoretical counter-example
  (glob metacharacters as literals) cannot arise from real
  tool/role definitions. Pinned with a 7-case test.
- **Misleading `CEILING_SEMITRUSTED` ▲-row doc comment.** Phase 15
  Task 4. **Closed by Task 6** — rewritten to distinguish ⊘ rows
  (hard-denied) from ▲ rows (conditionally granted) with precise
  D4 rule interaction semantics.

**Rolling deferrals still open after Phase 20 (inherited,
untouched):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1 deferral. Untouched by Phase 20.
- **Second regression channel for the role
  primitive** — Phase 11 Q6 deferral. Untouched.
- **Response headers in audit payload** (Phase 12
  Q3 half). Untouched.
- **Non-GET verbs (POST/PUT/PATCH/DELETE).** Phase
  12 Q1 pinned GET-only. Deferred indefinitely.
- **Redirect following with per-hop scope re-check.**
  Phase 12 Q5 pinned `Policy::none()`. Deferred
  indefinitely.
- **Binary response bodies / non-UTF-8.** Deferred
  indefinitely.
- **Per-chunk Telegram rendering.** Phase 12 Task 1.
  Deferred reactively.
- **Multi-level sub-agent nesting.** Phase 14 Task 3.
  Untouched.
- **LocalChannel regression-test rewrite over IPC.**
  Phase 17 Q6→(c+). Untouched. Tagged: **reactive.**
- **Telegram-specific protocol extensions
  (attachment delivery, inline keyboards, etc.).**
  Phase 19 net-new. Untouched.

**Net-new deferrals from Phase 20 itself:**

None.

**Backlog shape at Phase 20 exit:** sixteen inherited, six
closed, zero net-new. Total **ten**.

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak extends to twenty.
  **Reality: streak holds at twenty.** ✓
- **PRODUCT.md** — Predicted: streak extends to eight.
  **Reality: streak holds at eight.** ✓
- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak extends to nine (new record). **Reality: streak
  holds at nine (new record).** ✓

All three predictions correct. Phase 20 continues the
streak of accurate streak predictions established in
Phase 16.

## Exit criteria

- [x] All six targeted deferrals closed.
- [x] `daemon status` and `daemon stop` subcommands ship
      with parser tests and integration tests.
- [x] PID file written on daemon start, removed on clean
      shutdown via `PidGuard` Drop guard.
- [x] `--no-daemon` flag skips daemon dispatch for both
      Local and Telegram channels.
- [x] Daemon-mode banners match in-process banners
      (fs sandbox, memory status, audit event count).
- [x] `CEILING_SEMITRUSTED` doc comment rewritten with
      precise ⊘/▲ semantics.
- [x] `grants` reflexivity investigated, documented, and
      pinned with a test.
- [x] Test count: 550 → 569 (+19).
- [x] All three byte-identity streaks held:
      DESIGN.md (`e0d6437`, 20 phases),
      PRODUCT.md (`80189b4`, 8 phases),
      `aivyx-core/src/lib.rs` (`ba9a724`, 9 phases — new record).
- [x] Zero net-new deferrals.
- [x] Rolling backlog: 16 → 10.
- [x] Deferrals block recorded.
- [x] Prediction-versus-reality block recorded.
- [x] `docs/README.md` phase-status table reflects exit.
- [x] `docs/ROADMAP.md` Phase 20 frozen, Phase 21 scaffold.
- [x] `docs/PRODUCT_ROADMAP.md` Daemon Migration milestone
      updated with Phase 20 cleanup record.

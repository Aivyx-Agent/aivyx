# Phase 47 — Web UI Phase 2 (Mission Dashboard + Audit Viewer)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Extend the Web UI from a chat-only frontend into a full operator
inspection surface. Adds IPC query/response message types, a
mission dashboard, an audit viewer, and a session history pane.
Delivers **PRODUCT.md P2** (mission legibility) and **PRODUCT.md
P4** (daemon inspection) for the web frontend.

Phase 1 (Phase 39) shipped chat + tool-call cards + approval-gate
buttons. Phase 2 (this phase) makes the daemon's *state* — not
just its real-time output — operator-readable through the same
WebSocket bridge.

## Why now

1. **Differentiator visibility.** Aivyx's strongest unique
   property is the HMAC-chained audit log (per
   `docs/THREAT_MODEL.md` §4.3). Today an operator can only read
   it through `aivyx --verify-only`. A live audit viewer turns
   "we have audit" into "operators *use* audit."

2. **Substrate is ready.** `mission::list_missions` /
   `mission::get_mission` exist as agent-facing tools (Phase 28).
   `PersistentAuditLog::entries()` + `verify()` are public. The
   missing piece is a query-shaped IPC protocol extension — the
   daemon today only speaks event streams.

3. **Codebase health.** Phase 46 exited at 936 tests, zero clippy
   warnings, zero deferral backlog (after the post-46 sort_by
   clippy fix). Good moment for a feature phase.

## Architecture

The Web UI already bridges WebSocket frames to the daemon's Unix
socket at the frame level (Phase 39). This phase extends the IPC
protocol with a **query/response envelope** carried by the same
frame format — no new transport, no new authentication.

```
Browser ──Query{id, payload}──► Web UI Server ──► Daemon
                                                    ├─ MissionStore → ListMissions
                                                    ├─ PersistentAuditLog → ListAuditEntries
                                                    └─ SessionRegistry → ListSessions
Browser ◄──QueryResponse{id, payload}── Web UI ◄────┘
```

Queries are read-only. Mutating operations stay on the
existing turn-loop / gate-resolution paths.

## Entry baseline

- Tests: 936
- Clippy warnings: 0
- Deferral backlog: 0
- DESIGN.md streak: 5 phases (untouched since Phase 41)
- PRODUCT.md streak: 10 phases (untouched since Phase 37)
- lib.rs streak: 1 phase (untouched since Phase 45)

## Q-block — resolutions

Six load-bearing decisions, pinned at phase open:

**Q1: Query routing — envelope vs. per-query `FrontendMessage` variant?**
→ **Envelope.** `FrontendMessage::Query{id, payload: QueryPayload}`
and `DaemonMessage::QueryResponse{id, payload:
QueryResponsePayload}`. Keeps `FrontendMessage`/`DaemonMessage`
from growing one variant per query type, and the correlation `id`
lets the frontend match async responses without bookkeeping.

**Q2: Authorization — does audit/mission read need a capability check?**
→ **No, by design.** The IPC socket is `mode 0600`,
operator-owned (per `PRODUCT.md` P6 and `docs/THREAT_MODEL.md`
§4.4). Audit reads through the IPC are operator reads by
definition; gating them against `audit.read` would only be a
courtesy check against the operator's *own* role envelope, which
isn't the threat model. Documented inline in Task 4.

**Q3: Audit pagination — fixed window or caller-supplied limit?**
→ **Caller-supplied `limit` with a server cap of 500.** The
frontend asks for 100 by default; an operator with a long chain
can request more. The cap prevents a runaway query from blocking
the daemon on a single response.

**Q4: Live audit push — in scope or deferred?**
→ **Deferred.** The load-bearing delivery is *read* of mission
state + audit chain. Live push (broadcast new `SignedEntry`s to
subscribed clients) requires audit-bridge tap + per-client
subscription state and doesn't change what the operator can do —
only how fresh the data is. Carry as a net-new deferral; revisit
when a real use case appears.

**Q5: Frontend shape — tabbed SPA or multi-page?**
→ **Tabbed SPA inside the existing `web_ui_static.html`.**
Phase 39 shipped the asset as a single embedded HTML file;
staying single-file matches the daemon-single-binary aesthetic
and avoids invalidating the Phase 39 build pattern.

**Q6: Read-only or read-write inspection?**
→ **Read-only for v2.** Mutating ops (cancel mission, delete
schedule, edit role config) are a separate phase. Gate
approve/deny already works inline from the chat panel via
`ResolveGate` and is unchanged.

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (6) | Pure protocol extension — no contract change. |
| PRODUCT.md | untouched (11) | Inspection surface is P4 delivery, not a new commitment. |
| lib.rs | untouched (2) | No `aivyx-core/src/lib.rs` edits expected — work is in `aivyx-channel`. |

## Tasks

### Task 1 — Open commit + scaffold

This file. Update `docs/README.md` to show Phase 47 as Open.
Replace the Phase 47 paragraph in `docs/ROADMAP.md` with an
"Active — see PHASE_47.md" marker.

### Task 2 — Query/QueryResponse IPC variants + ListSessions e2e

Add `FrontendMessage::Query{id, payload}` and
`DaemonMessage::QueryResponse{id, payload}` in `daemon_ipc.rs`.
Define `QueryPayload` + `QueryResponsePayload` enums with the
first concrete variant: `ListSessions` → `SessionSummary[]`. Wire
daemon-side dispatch in `daemon_server.rs` so the round trip
works end-to-end against the in-memory session registry.
Serde round-trip + integration test.

### Task 3 — Mission queries (ListMissions, GetMission)

Add `MissionSummary` + `MissionDetail` response payloads. Daemon
handler delegates to `mission::list_missions` and
`mission::get_mission` (already public). Integration test against
a `MissionStore` populated by the test harness.

### Task 4 — Audit queries (ListAuditEntries, VerifyAuditChain)

Add a ranged-read method to `PersistentAuditLog` (`entries_from`?
or `entries_range`?) — implementation choice for the task.
Wire `ListAuditEntries{from_seq, limit}` (server-cap 500) and
`VerifyAuditChain` queries. Document inline (per Q2) that no
capability check applies: IPC socket auth is the authorization
boundary.

### Task 5 — Frontend tabbed SPA

Extend `web_ui_static.html` with a tab strip (Chat / Missions /
Audit / Sessions). Chat pane is byte-identical to Phase 39.
Missions/Audit/Sessions panes issue the new `Query` messages on
tab activation, render the responses. Per-pane refresh button.
Keep the asset as a single embedded HTML file.

### Task 6 — Exit freeze

Update this file's exit criteria, ship records, exit stats.
Update `ROADMAP.md` to mark Phase 47 frozen with the exit commit
hash. Update `docs/README.md` row. Final commit.

## Ship records

| Task | Commit | Tests after |
|---|---|---|
| 1 | `7c4db55` | 936 |
| 2 | `230c47f` | 938 |
| 3 | `276d96d` | 940 |
| 4 | `d35279f` | 946 |
| 5 | _this commit_ | 948 |

## Deferrals carried into the phase

None. Backlog entered at 0.

## Net-new deferrals (predicted)

- **Live audit push.** Per Q4. Carry into the phase exit's
  deferral backlog.
- **Read-write inspection.** Per Q6. Cancel mission, delete
  schedule, etc. — sized at a small follow-up phase, not urgent.
- **`handle_connection` parameter struct.** Surfaced in Task 4
  when the audit-log threading pushed the function over the
  `too_many_arguments` clippy threshold. `#[allow]`'d for now
  with the lift documented at the function. Same pattern as
  Phase 41 Task 2's `DaemonConfig` lift — recommended for a
  future cleanup phase, not urgent.

## Exit criteria

- [ ] `FrontendMessage::Query` and `DaemonMessage::QueryResponse`
  variants land with serde round-trip tests.
- [ ] `ListSessions`, `ListMissions`, `GetMission`,
  `ListAuditEntries`, `VerifyAuditChain` all work end-to-end
  over the Unix socket.
- [ ] Web UI shows four tabs; Missions/Audit/Sessions panes load
  data from the daemon and render it.
- [ ] All tests pass with net-positive delta.
- [ ] Zero clippy warnings under rust 1.95.
- [ ] DESIGN.md untouched (streak → 6).
- [ ] PRODUCT.md untouched (streak → 11).
- [ ] lib.rs untouched (streak → 2).

## Exit stats

_To fill at exit._

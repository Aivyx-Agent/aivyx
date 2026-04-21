# Amendment A7 — Protocol Version Negotiation

**Date:** 2026-04-21
**Phase:** 41 (Daemon Hardening & Error Typing)
**Supersedes:** Extends A1 (Daemon IPC Protocol). Additive only.
**Closes:** Sole remaining deferral (protocol versioning).

---

## What changed

A1 documented the IPC protocol with `PROTOCOL_VERSION = "0.1"`
but deferred version negotiation — the constant existed with no
wire-level handshake to validate compatibility between client
and daemon. Phase 41 closes this deferral.

## The negotiation protocol

After receiving `DaemonReady`, a frontend **may** send:

```json
{ "type": "ProtocolNegotiation", "version": "0.1" }
```

The daemon responds with one of:

```json
{ "type": "ProtocolAccepted", "version": "0.1" }
```
```json
{ "type": "ProtocolRejected", "supported": ["0.1"] }
```

### Semantics

- **Optional handshake.** Negotiation is not required. Clients
  that omit `ProtocolNegotiation` are assumed to speak v0.1.
  This preserves backward compatibility with existing frontends.

- **v0.1 always accepts.** The daemon accepts any version string
  for the v0.1 protocol. This is forward-compatible: when v0.2
  ships, the daemon can reject unknown versions. The
  `ProtocolRejected` variant exists in the wire format now so
  clients can handle it without protocol changes.

- **Position in message sequence.** `ProtocolNegotiation` is
  sent after `DaemonReady` and before `StartSession`. It may
  also be sent at any point during the connection (the daemon
  handles it as a regular `FrontendMessage` in the dispatch
  loop), though the intended usage is once at connection start.

## Message type additions

**`FrontendMessage`** gains:
- `ProtocolNegotiation { version: String }`

**`DaemonMessage`** gains:
- `ProtocolAccepted { version: String }`
- `ProtocolRejected { supported: Vec<String> }`

**`DaemonEnvelope`** gains corresponding variants for client
demuxing.

## How A1 should be read after this amendment

A1's message envelope tables should be read as extended:

- `FrontendMessage`: add `ProtocolNegotiation` to the list.
- `DaemonMessage`: add `ProtocolAccepted`, `ProtocolRejected`.
- `DaemonLifecycleEvent`: add `RecoveryNotice` (Phase 41 Task 4).

The deferral of protocol versioning (backlog item 1) is closed.
Deferral backlog is now **0**.

---

## Traceability

| Phase | What shipped | Commit |
|---|---|---|
| Phase 41 | Protocol negotiation messages, always-accept v0.1, e2e test | *(this phase)* |

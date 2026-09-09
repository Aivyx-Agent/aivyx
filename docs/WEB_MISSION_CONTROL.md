# Web Mission-Control GUI — a Rust/WASM Web UI (Chapter M)

> **Status:** design contract. This is the spec Chapter M scaffolds from
> (mirrors `docs/DAEMON_TEAMS.md` for Chapter L).
>
> Chapter L made the daemon **run Nonagon teams** — durable, gate-pausable
> missions over a poll-based IPC surface (`TeamRun` / `TeamRunGoal` /
> `TeamMissionList` / `TeamMissionStatus` / `ResolveTeamGate`), already driven
> from the CLI (`aivyx-pa team …`) and the TUI Missions panel. Chapter L's doc
> named this chapter as the payoff it unblocks: a **browser** Mission-Control
> GUI.
>
> Chapter M replaces the hand-written Web UI with a **Rust → WASM** app
> (Dioxus), and extracts the daemon's wire protocol into a shared, wasm-clean
> `aivyx-ipc` crate so the browser and the daemon **cannot drift** from the
> wire format — the same correctness-first posture the rest of the codebase
> takes with capability types and the HMAC audit chain.

---

## 0. Decisions (locked at scope time)

| Decision | Choice | Why |
|---|---|---|
| Front-end stack | **Rust → WASM** | Share the IPC types with the daemon; no JS mirror to drift. |
| Framework | **Dioxus** (web renderer, `dx` bundler) | Ergonomic RSX, first-class WASM, app-like dashboard. |
| Type sharing | **Full `aivyx-ipc` protocol crate** | One source of truth for the *whole* wire protocol, not just missions; future-proofs every client. |
| UI scope | **Unify** — the WASM app replaces the existing chat Web UI | One Rust app for the whole browser surface (chat + missions + the read-only panels), not a second page bolted on. |
| Mission transport | **Poll** `TeamMissionList` (push later) | Consistent with the TUI / CLI / `LoopStatus`; the broadcaster-push option is additive. |
| Network posture | **Loopback only** (`127.0.0.1`), unchanged | Same trust boundary as today's Web UI; no new auth surface. |

---

## 1. What exists today

- **A hand-rolled Web UI server.** `aivyx-channel/src/web_ui.rs` binds
  `127.0.0.1:<web_ui_port>` on the **same TCP listener** as the webhook
  receiver, serves an embedded **2,879-line** `web_ui_static.html` at `GET /`,
  and upgrades `GET /ws` to a WebSocket. It deliberately avoids `hyper`
  (peeks the request line, hands `/ws` to `tokio-tungstenite`).
- **A transparent WS ↔ IPC bridge.** `handle_websocket` opens the daemon's
  Unix socket, performs the `StartSession { frontend_type: Web }` handshake,
  forwards `SessionStarted`, then **relays JSON both ways**: the browser sends
  `FrontendMessage` JSON, receives `DaemonMessage` JSON. *The team-mission IPC
  is therefore already reachable from a browser* — Chapter M is mostly a
  front-end + a type-sharing refactor, not new daemon plumbing.
- **A broadcaster for server-push.** `notify_webui.rs::WebUiBroadcaster` (a
  `tokio::sync::broadcast`) already fans desktop-notification frames out to
  every WS connection. The mission feed can reuse it for push **later**; M
  ships poll first.
- **A rich (but static) page.** The current HTML already references missions,
  audit, and approve/reject — so "unify" means **parity with a full page**,
  not just a chat box. It hand-writes JSON matching the Rust enums' serde tags
  (`#[serde(tag = "kind")]` / `"type"`) and hand-mirrors the response shapes —
  exactly the drift Chapter M removes.
- **The IPC enums.** `daemon_ipc.rs`: `FrontendMessage`, `DaemonMessage` /
  `DaemonEnvelope`, `QueryPayload`, `QueryResponsePayload`,
  `DaemonLifecycleEvent`, `StreamEventPayload`, `FrontendType`,
  `IpcAttachment`, plus ~20 summary/data structs, and the length-prefixed
  `encode_frame` / `decode_frame` / `FrameError` codec.

---

## 2. The crux — a wasm-clean `aivyx-ipc`, carved out without daemon drift

The framework is the easy part. The crux (Chapter M's analogue of L's
checkpoint/resume) is **extracting the entire wire protocol into a crate that
compiles to `wasm32-unknown-unknown`**, while the daemon keeps using the exact
same types and **every existing test stays green**.

The blocker is the dependency graph:

```
aivyx-capability   wasm-clean ✓  (serde + globset only)
aivyx-core         NOT wasm-clean ✗  (tokio, reqwest, storage, libc, jsonschema)
aivyx-team         NOT wasm-clean ✗  (→ aivyx-core, aivyx-llm)
aivyx-channel      NOT wasm-clean ✗  (the daemon: storage, tokio net, redb, …)
```

Today the wire types **live inside those heavy crates** (e.g. `MissionPlan` in
`aivyx-team`, `TeamMissionRecord` in `aivyx-channel`, `LoopRunState` /
`Story` / `EffectivePersona` in their feature modules). A WASM crate cannot
depend on them. So the pure **data** must be separated from **behavior**:

- The `struct`/`enum` definitions + their serde derives + pure helper `impl`s
  move to wasm-clean crates.
- Behavior that needs the daemon (storage I/O, async, providers) stays in the
  heavy crate as inherent or free functions over the moved types.

This is mechanical but broad. The discipline that keeps it safe: **move, then
re-export.** Each heavy crate re-exports the moved types from their new home
(`pub use aivyx_ipc::…;`), so call sites and tests are unchanged — the daemon
compiles and behaves identically; only the *definitions' location* moved.

**Audit gate (M.1 pre-flight):** before moving anything, grep every wire type
for references to non-wasm-clean types — most importantly `aivyx-core` ids
(`SessionId` / `AgentId` / `TurnId`) and any embedded behavior type. The
encouraging signal: wire ids are already `String` on the summaries. The likely
offender is `StreamEventPayload` (chat streaming) — audit it first.

---

## 3. Crate topology (target)

```
aivyx-capability      (unchanged, already wasm-clean)
        ▲
aivyx-team-types      NEW, wasm-clean: MissionPlan, Step, StepKind, GateMode,
   │  │                TeamConfig, TeamMember, DialogueConfig, MissionStatus,
   │  │                MissionReport, plan validation. (→ aivyx-capability)
   │  ▼
   │ aivyx-ipc        NEW, wasm-clean: the full protocol — FrontendMessage,
   │  ▲                DaemonMessage/Envelope, QueryPayload, QueryResponse-
   │  │                Payload, lifecycle + stream events, every summary/data
   │  │                struct (incl. TeamMissionRecord/View/Phase, LoopRunState,
   │  │                Story, EffectivePersona…), and encode_frame/decode_frame.
   │  │                (→ aivyx-team-types, aivyx-capability, serde, serde_json)
   ▼  ▼
aivyx-team ──▶ re-exports aivyx-team-types       (daemon side, unchanged API)
aivyx-channel ─▶ re-exports aivyx-ipc            (daemon side, unchanged API)
        ▲
aivyx-web             NEW, the Dioxus WASM app. (→ aivyx-ipc, dioxus,
                       gloo-net/web-sys for WebSocket)
```

- `aivyx-team-types` exists because `MissionPlan` rides on the wire
  (`TeamRun { plan }`) yet lives in the un-wasm-able `aivyx-team`. Splitting
  the pure mission/team **data** out lets both `aivyx-ipc` and `aivyx-team`
  share it. (If the audit shows the set is tiny, it may fold directly into
  `aivyx-ipc` instead — an M.1 call.)
- `aivyx-web` never depends on `aivyx-core`/`-channel`/`-team`; only on
  `aivyx-ipc` (+ `aivyx-capability` transitively). That is the invariant that
  keeps it wasm-buildable.

---

## 4. The Dioxus app (`aivyx-web`)

A single-page app reaching **parity with the current Web UI plus live Mission
Control**. It owns a model/update loop analogous to the TUI's (`aivyx-tui`),
but rendered as Dioxus components and fed over the WebSocket.

- **Transport.** One WebSocket to `/ws`. The bridge already does the
  `StartSession(Web)` handshake server-side and relays JSON, so the app:
  serializes `aivyx_ipc::FrontendMessage` → JSON → WS; parses WS → JSON →
  `aivyx_ipc::DaemonMessage`. **No hand-written wire JSON** — the shared types
  serialize identically to what the daemon expects.
- **Views (parity target).** Chat (submit input, stream events, resolve the
  single-agent gate), **Missions** (the new headline), and the read-only
  panels the page already implies (Audit, Dashboard, Tools) — ported as the
  shared summary types make them nearly free.
- **Mission Control view** (the chapter's point), mirroring the L.6 TUI panel:
  - a **feed** polled from `TeamMissionList` (records → rows; reuse the
    `TeamMissionRecord::to_view()` projection, now in `aivyx-ipc`),
  - a **detail** pane (phase, per-step state, progress),
  - a **new-mission** form → `TeamRunGoal { goal, config? }` (or `TeamRun`
    with an explicit plan), with optional pack selection,
  - **approve / reject** affordances on an `AwaitingApproval` mission →
    `ResolveTeamGate`.
- **Styling.** The Aivyx PA palette (port `aivyx-tui/src/palette.rs` colors) so
  the browser and terminal read as one product.

---

## 5. Serving + build

- **Bundle.** `dx bundle` (Dioxus) emits `index.html` + an app `.wasm` + a JS
  glue `.js` (+ assets). These are embedded into the daemon binary via
  `include_bytes!` / `include_dir!`, replacing the single
  `web_ui_static.html` string.
- **Serving.** `web_ui.rs` grows from "serve one HTML" to a tiny **static
  router**: `GET /` → `index.html`; `GET /<app>.wasm` → the wasm with
  `Content-Type: application/wasm`; `GET /<app>.js` → the glue with
  `text/javascript`; assets likewise. `/ws` is unchanged. Still no `hyper`.
- **Build orchestration.** The wasm build is **not** wired into `cargo build`
  (it must not force a wasm toolchain on every contributor). Instead: a
  `justfile`/`Makefile` target (`build-web`) runs `dx bundle`, and a CI job
  builds it on the `wasm32-unknown-unknown` target. The embed uses a checked-in
  **fallback page** (a "run `just build-web`" notice) when `dist/` is absent,
  so a plain `cargo build` of the daemon always succeeds.

---

## 6. Phase plan

| Phase | Deliverable |
|---|---|
| **M.0** | This design contract. |
| **M.1** | `aivyx-team-types`: extract the pure mission/team data out of `aivyx-team` (wasm-clean); `aivyx-team` re-exports — no API/behavior change, all tests green. Confirm `aivyx-capability` builds for `wasm32`. |
| **M.2** | `aivyx-ipc`: move the **full** protocol + frame codec out of `aivyx-channel` (wasm-clean); `aivyx-channel` re-exports. The crux — do it in slices behind the wasm-compat audit (StreamEventPayload first). Daemon byte-for-byte unchanged. |
| **M.3** | `aivyx-web` skeleton (Dioxus): WS client, the IPC types, connect + handshake, a **read-only Mission feed** polled from `TeamMissionList`. Builds to wasm via `dx`. |
| **M.4** | Mission Control interactions: detail pane, new-mission form (`TeamRunGoal`/`TeamRun`), approve/reject (`ResolveTeamGate`); plus the **chat** view (submit + stream + single-agent gate) for parity. |
| **M.5** | ✅ Serve the bundle from the daemon: `build.rs` embeds `crates/aivyx-web/dist/` (empty when unbuilt) → `web_ui.rs` static router serves `/`, `/<app>.wasm` (`application/wasm`), `/<app>.js`, … `/ws` unchanged. **Fallback at `/` is the legacy `web_ui_static.html`** while the bundle is unbuilt / until WASM chat parity (M.6) — so a plain `cargo build` needs no wasm toolchain and the browser never regresses. `justfile` (`build-web` / `check-web` / `clean-web`) + a CI wasm compile-check. (Retiring `web_ui_static.html` deferred to M.6 with chat parity.) |
| **M.6** | ✅ (partial) WASM **chat view** + Missions/Chat tabs (M.6a), Aivyx PA-palette styling, connecting/streaming states. **Scope correction:** the legacy `web_ui_static.html` has ~8 panes (chat, missions, audit, sessions, memory, learning, proposals, notifications); porting all of them to WASM is far larger than a slice. So it is **kept, not retired** — served at `/classic` (the new app links to it) so building the bundle never regresses a pane. The remaining inspection panes' WASM port (audit/memory/learning/proposals/notifications/sessions) + live mission **push** via `WebUiBroadcaster` are a documented **follow-on (M.7+)**, not blocking the Mission-Control + Chat GUI this chapter set out to deliver. |

~7 phases, Chapter-L-sized. M.1 + M.2 (the extraction) are the bulk and the risk.

---

## 7. Invariants

- **No daemon drift.** The type extraction is move-and-re-export only: the
  daemon's public APIs, wire bytes, and tests are unchanged. A passing daemon
  test suite after M.2 is the proof.
- **`aivyx-web` is wasm-pure.** It depends on `aivyx-ipc` (+ `aivyx-capability`)
  and nothing heavier — never `aivyx-core` / `-channel` / `-team`. CI builds it
  on `wasm32-unknown-unknown` to enforce this.
- **One source of wire truth.** Browser and daemon serialize the *same* Rust
  types; there is no hand-maintained JSON mirror. Adding an IPC variant updates
  both sides at once.
- **Loopback-only, no new trust surface.** The server stays bound to
  `127.0.0.1`; the WS↔IPC bridge and its session handshake are unchanged. The
  capability/audit boundary is the daemon, exactly as for the CLI/TUI.
- **`cargo build` needs no wasm toolchain.** The daemon embeds a fallback when
  the bundle isn't built; only `just build-web` / CI needs `dx` + the wasm
  target.

---

## 8. Open questions (to resolve in-phase, not blocking M.0)

- **`aivyx-team-types` vs. fold-into-`aivyx-ipc`** — decide in M.1 once the
  pure set's size is known.
- **`StreamEventPayload` shape** — if it embeds `aivyx-core` types, define a
  wasm-clean wire form during the M.2 audit (it already crosses the WS today,
  so a serializable shape exists).
- **`dx` vs `trunk`** for bundling — both work; default `dx` (Dioxus-native),
  revisit if asset/embed ergonomics favor `trunk`.
- **Mission push** (M.6) — extend `WebUiBroadcaster` with a mission-changed
  frame, or keep poll. Poll ships first regardless.

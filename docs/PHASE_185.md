# Phase 185 — Terminal TUI Foundation

**Chapter I, phase 1** — the first interface-axis phase. Today
the local interactive surface is a plain line-based REPL
(`run_session`): read a line, send it to the daemon, print the
streamed render, repeat. Phase 185 ships a real **terminal UI** —
a `ratatui` frontend over the daemon IPC with a scrollable chat
pane, a status bar, an input line, and clean keybindings — so the
keyboard experience is an *application*, not a print loop.

## Architecture — a frontend, like every other interface

The daemon is the agent; the TUI is just another **frontend
client** over the local Unix-socket IPC (exactly like the Web UI
and the REPL). It `DaemonSession::connect`s (auto-spawning the
daemon if needed), submits input, and renders the same
`StreamEventPayload` stream the REPL renders to stdout — only
into ratatui widgets instead of a byte sink. No daemon, agent,
capability, or audit change: this is a pure render + interaction
layer.

## The deliberate dependency break

A real TUI needs a terminal-UI library. **`ratatui` + `crossterm`**
(operator-confirmed) land as the **first new workspace
dependencies in a long time** — consciously ending the
zero-new-dep streak held across the entire 172–184 run. The
trade is right (a load-bearing, widely-audited, pure-Rust stack
for a feature that genuinely requires it), and it is
**quarantined to a new `aivyx-tui` crate**, so the substrate
crates (`aivyx-core`, `aivyx-capability`, `aivyx-storage`,
`aivyx-crypto`, …) stay dependency-clean. `aivyx-channel`'s
binary depends on `aivyx-tui` only to launch it.

## Non-breaking by design

The TUI is **opt-in** via `aivyx tui`. The REPL stays the
**default** and the non-TTY / scripting / piped path — the TUI
requires a real terminal, and the REPL must keep working for
automation. Once the TUI proves itself it can become the default
in a later phase; this foundation never removes the REPL.

## Design

- **`aivyx-tui` crate** — `ratatui` + `crossterm`, depends on
  `aivyx-channel` for `DaemonSession` + the IPC types.
- **A pure TUI model** (`AppState` + an `update(Msg) -> AppState`
  reducer) — the testable core. `AppState` holds the chat
  history (rendered lines), the input buffer, the scroll offset,
  the status (role / daemon / working), and any pending approval
  gate. `Msg` is a key event or a daemon turn result. The
  ratatui *rendering* of `AppState` is operator-verified (no
  terminal in CI) — the same operator-verification posture as the
  sandbox / OAuth dance.
- **`StreamEventPayload` → chat lines** — a mapping that reuses
  the existing `render_for_cli` text content, turned into the
  model's line list (operator messages, agent text, tool-call
  breadcrumbs).
- **Approval gates in-TUI** — the REPL handles `ApprovalGate`
  inline; the TUI renders it as an approve/reject prompt so gated
  turns work in the TUI too.
- **Keybindings** — `Enter` send, `PgUp`/`PgDn` (and arrows)
  scroll, `Esc` / `Ctrl-C` cancel the in-flight turn, `Ctrl-Q`
  quit. A help line in the status bar.
- **Turn model (foundation)** — uses the existing
  collect-then-return `submit_input` with a clear **"working…"**
  status while the turn runs; **live token streaming** (a
  streaming-receive `DaemonSession` variant) is the natural
  Chapter I follow-on, not part of this foundation.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 184's frozen hash (`ff0463d`).
2. **`aivyx-tui` crate + the pure model.** The crate skeleton
   (ratatui/crossterm deps, workspace member) + `AppState` +
   `update` + the `StreamEventPayload` → lines mapping. Tests for
   the reducer: input editing, submit clears + appends, scroll
   bounds, working/idle status, gate-pending set/clear, the
   event→line mapping.
3. **The terminal driver + render.** ratatui init/teardown (raw
   mode, alternate screen, panic-safe restore), the layout
   (chat pane + status bar + input line), and the crossterm event
   loop feeding the reducer. Operator-verified rendering; the
   panic-safe restore + the layout are the deliverables. A
   headless smoke test of the render-to-buffer where ratatui's
   `TestBackend` allows.
4. **`aivyx tui` command + daemon integration.** The `CliMode::Tui`
   parse + dispatch; `DaemonSession` connect/auto-spawn, submit,
   gate handling, cancel, quit. Tests: CLI parse; the
   session-glue seams that are pure.
5. **INSTALL + exit + Frozen.** INSTALL section (the TUI, the
   keybindings, opt-in vs the REPL, the new deps); exit doc;
   README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 21 → **22**. A new
  frontend + a new quarantined crate — the capability / trust /
  audit contract is untouched, and P5 explicitly anticipates a
  TUI. (Adding a workspace member is additive structure, as the
  Chapter F productivity crates were.)
- **PRODUCT.md** — **Will hold.** Streak: 75 → **76**. P5
  frontend territory; not a new commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 21 →
  **22**. The TUI is `aivyx-tui` + the bin; `aivyx-core` is
  untouched.
- **Zero-new-dependency streak** — **BREAKS, deliberately.**
  `ratatui` + `crossterm` are the first new workspace deps since
  well before Phase 172. Flagged loudly, justified, quarantined
  to `aivyx-tui` so the substrate stays clean. This is the one
  tracked discipline this phase consciously reverses.

## Exit criteria

- [ ] `docs/PHASE_185.md` + README row + Phase 184 backfill — T1.
- [ ] `aivyx-tui` crate + the pure `AppState`/`update` model +
  event→line mapping — T2.
- [ ] The ratatui terminal driver with panic-safe restore — T3.
- [ ] `aivyx tui` connects, submits, handles gates, cancels,
  quits; REPL still the default — T4.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] **Exactly two** new workspace deps (`ratatui`, `crossterm`),
  quarantined to `aivyx-tui`.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+20`. *(Component-priced: the
  pure reducer is the dense part — key-edit / submit / scroll /
  status / gate transitions + the event→line mapping. The
  terminal rendering + daemon glue are operator-verified, not
  unit-tested.)*

## Honest scope risks at sign-off

- **The rendering + terminal interaction are operator-verified.**
  No real terminal in CI; the pure model is unit-tested, the
  visual experience is verified on the operator's host (the
  threat-model / sandbox pattern). `TestBackend` covers what it
  can headlessly.
- **No live token streaming in v1** — collect-then-render with a
  "working…" status. Live streaming needs a streaming-receive
  `DaemonSession` variant; it is the natural next Chapter I phase,
  flagged not skipped.
- **The dependency break is real** — two new crates enter the
  tree. Quarantined, but the substrate-dep-clean invariant is now
  load-bearing on the crate boundary (a future audit should
  confirm `aivyx-tui` is the only ratatui consumer).
- **Opt-in, not default** — discoverability rests on docs + the
  init flow until a later phase promotes it.

## Prediction vs reality

_(Filled at exit.)_

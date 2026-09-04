# Phase 191 — Daemon-side automatic alert dispatch for tool processes — design

**Status:** Approved, ready for planning.

## Motivation

Phase 191 had nothing pre-named in `docs/ROADMAP.md`'s Chapter I or Chapter
G to ground against. Chapter G's "Chapter G #2 candidates" (from Phase
125's own exit) named two: lightweight budget tracking and
`health.check.remove` + automatic alert dispatch. Grounding found both
budget tracking (CRUD + trend + categories, Phases 143/144/147/149/150) and
`health.check.remove` (Phase 147) already fully shipped — the roadmap
wording was stale, same class of drift Phases 187/188/189 each hit.

What's genuinely still open is the other half of that second candidate.
Two docs say the identical thing, verbatim, and have since Phase 125:
`crates/aivyx-toolkit/src/tools/health_check.rs:14-32` and
`docs/INSTALL.md:4414-4491` — "the polling loop records state transitions;
the agent composes alerts. Substrate-minimal per Phase 125 — no daemon-side
automatic alert dispatch (deferred to Phase 126+)." Today's real
workaround: the operator configures an hourly cron whose prompt tells the
*agent* to call `health.check.recent_changes` and manually decide whether
to call `notify.send`. This requires the cron to exist, the turn to fire,
and — per `INSTALL.md`'s own finding — a cloud-provider model, since local
Ollama "won't reliably make the multi-tool call sequence." A health check
going down can silently produce no notification at all. `INSTALL.md`
itself sketches the intended fix: "Phase 126+ may add a daemon-side IPC
hook for tool processes to dispatch notifications directly." This phase
builds that hook, generalized (not health-check-specific in the wire
protocol), with health-check as the one real, wired-up consumer.

## Architecture

**Wire protocol** (`crates/aivyx-tool/src/wire.rs`). `ToolToDaemon`
currently has 4 variants (`ToolRegister`, `ToolEvent`, `ToolResult`,
`ToolError`), every one routed by `bridge.rs`'s reader loop through a
`pending: HashMap<call_id, ...>` lookup tied to a daemon-initiated
`InvokeTool`. There is no existing push channel — a tool process cannot say
anything unprompted today. A new variant:

```rust
DispatchNotification {
    target: String,
    message: String,
    subject: Option<String>,
}
```

deliberately carries no `call_id` — it isn't a response to any invocation,
it fires from a tool process's own background task. Field shape mirrors
`notify.send`'s own three fields exactly (`target`, `message`, `subject`),
so it's a familiar addition to anyone who already knows that tool.

**Bridge routing** (`crates/aivyx-tool/src/bridge.rs`). **Correction from
an earlier draft of this design**: `bridge.rs` cannot call
`NotifyDispatcher::dispatch` directly — `aivyx-tool` and `aivyx-channel`
are dependency-free siblings today (verified via every crate's
`Cargo.toml`: neither depends on the other), and `aivyx-tool` is a
generic, channel-agnostic substrate consumed by several tool-process
crates that have no business depending on the full channel/daemon stack
(`aivyx-gmail`, `aivyx-calendar`, `aivyx-drive`, etc. all depend on
`aivyx-tool` alone). Adding `aivyx-channel` as a dependency of
`aivyx-tool` would break that boundary.

Instead, `ToolProcessBridge` (the real struct name in `bridge.rs`) gains a
new, optional injected dependency for handling `DispatchNotification`
frames — the exact Rust shape (an async trait object defined in
`aivyx-tool` and implemented for `NotifyDispatcher` from the constructing
crate, vs. a boxed-future callback) is a plan-writing-stage decision, not
fixed here; either keeps `aivyx-tool` free of any dependency on
`aivyx-channel`, which is the actual constraint. Absent (the default),
existing tool processes that never send `DispatchNotification` are
unaffected. The reader loop's new `DispatchNotification` arm skips the
`pending`-map lookup entirely (there's no call_id to look up) and invokes
the injected sink if present. The daemon's own startup code —
`crates/aivyx-cli/src/bin/aivyx.rs`, the one real construction site for
`ToolProcessBridge` besides `aivyx-mcp`'s unrelated stdio bridge — wires
this to the real `NotifyDispatcher` when it spawns the toolkit's tool
process, since that's the one place that already holds both the bridge and
the dispatcher (`aivyx-cli` depends on both `aivyx-tool` and
`aivyx-channel` already).

**Capability gating.** Reuses the exact existing precedent:
`ToolDescriptor.required_scope` is declared per-tool and checked against
the active role's capability envelope at `ToolHello`/`ToolRegister` time
(`docs/TOOL_SDK.md`'s documented handshake). A new scope, `notify.dispatch`,
gets added to `aivyx-capability`'s `KNOWN_BASES` and declared by
`aivyx-toolkit` alongside its existing `health.read`/`health.write`/
`task.read`/`task.write` scopes — same mechanism, no new opt-in system. This
is not optional: D4 (`DESIGN.md`) requires every action get a capability
check, and a push channel that bypassed it would be a real architectural
violation, not a shortcut worth taking to save one enum variant + one scope
string.

**Target selection.** A new `[toolkit] default_notify_target` field in
`~/.aivyx/tool-processes/toolkit/config.toml` — one target for the whole
tool process. `notify_dispatcher.rs`'s `dispatch()` takes an explicit
`target_name` (same shape `notify.send` uses); there's no "broadcast to
every configured target" convenience at that layer, and building one is out
of scope. Per-watcher targets (a different target per `health.check.add`
call) are explicitly deferred — real candidate for later if pressure
surfaces, not needed to close today's actual gap.

**Trigger site.** `health_store.rs`'s `record_check`
(`crates/aivyx-toolkit/src/health_store.rs:381-424`) already detects real
state transitions only — a `Transition` is pushed to the ring buffer iff
the watcher had a prior check *and* its ok-flag flipped (confirmed by the
existing `record_check_no_state_change_no_transition` test). This gives
natural anti-spam for free: a watcher steady in "down" for hours doesn't
re-fire on every poll interval, only real flips produce a transition.
`record_check`'s return type changes from `Result<(), HealthStoreError>` to
`Result<Option<Transition>, HealthStoreError>`. `health_polling.rs`'s loop
(the only caller) composes a message from the returned `Transition` and
sends `DispatchNotification` when one fires. Both directions notify (down
*and* recovered) — matches what the current agent-mediated recipe already
does (`INSTALL.md`'s cron prompt says "for each change," not just
failures); this phase makes existing behavior automatic, not a behavior
change.

## Message content and error handling

Message composed in `health_polling.rs` from the `Transition` (watcher
name, direction, status code, timestamp) — e.g. `"Health watcher
'my-service' went DOWN (status 503)"` / `"...RECOVERED"`. No new formatting
infrastructure.

- **`default_notify_target` unset**: the polling loop logs and skips —
  no dispatch attempted. Matches the toolkit's established
  degrade-gracefully pattern (e.g. `web.search` failing open with a clear
  error rather than crashing).
- **Target configured but unknown to the daemon, or the backend send
  itself fails**: logged via the tool process's existing log-forwarding
  path (`ToolEventPayload::Log`) — never panics, never aborts the polling
  loop. A failed notification must not take down health monitoring itself.

## Testing

- `health_store.rs`: extend the existing `record_check` test suite for the
  new `Option<Transition>` return — both transition directions, plus the
  already-covered "no transition on first poll" / "no transition when
  unchanged" cases (don't replace, extend).
- `bridge.rs`: a test that a `DispatchNotification` frame with no
  `call_id` routes correctly through the new arm and isn't silently
  swallowed by the `pending`-map lookup path.
- Capability test confirming `notify.dispatch` is rejected at handshake
  when not granted, mirroring the existing `required_scope` rejection
  tests for other toolkit scopes.
- No live-daemon or Playwright verification needed — this is backend Rust
  logic, not CSS/frontend rendering.

## Out of scope

- Per-watcher notify targets.
- Any tool process besides `aivyx-toolkit` actually using the new wire
  capability — it's general-purpose by construction (the protocol variant
  isn't health-check-specific), but this phase wires up exactly one real
  consumer. Speculative future consumers are YAGNI.
- Rate-limiting/dedup beyond what `record_check`'s existing
  transition-only detection already provides.
- The still-outstanding `run_multi_tool_subprocess` harness-lift tech debt
  (Phase 123's own SDK-validation finding, noted separately in
  `INSTALL.md`) — a real but unrelated piece of debt, not bundled here.

## Documentation correction bundled in

`docs/INSTALL.md`'s "What Phase 125 deliberately leaves to follow-on
phases" section (lines 4483-4499) gets corrected: the "No `health.check.remove`
tool" bullet removed (shipped Phase 147, stale since), and the "No
automatic alert dispatch" bullet updated to point at this phase instead of
"Phase 126+."

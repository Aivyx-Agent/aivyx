# Aivyx Roadmap

A living list of planned phases beyond the one currently active.

**This is not a contract.** Phase goals are revised at every phase
exit based on what the prior phase uncovered. Phase numbering is
loose — if Phase N's exit reveals that Phase N+2 should come before
Phase N+1, we reorder here and it costs one commit, not an amendment.

For the locked design contract, see [`../DESIGN.md`](../DESIGN.md).
For the current active phase, see the PHASE_N.md file listed in
[`README.md`](README.md). This document answers the question *"what
comes after the current phase?"* — nothing more.

## How this document is maintained

- At **every phase exit**, the outgoing phase's entry here is removed
  (it's now frozen in `PHASE_N.md`), and the next phase's one-line
  intent is refined with whatever the exiting phase learned.
- At **every phase entry**, a new `PHASE_N.md` is scaffolded from
  this document's entry for that phase, then the entry is replaced
  with a pointer like *"Active — see PHASE_N.md."*
- **Phase goals here are one paragraph maximum.** If an entry grows
  task lists or open questions, it has outgrown the roadmap and
  belongs in its own PHASE_N.md — which means that phase is probably
  ready to open.

## Channel Activation Milestone — operator verification across all channels

**Status:** Scheduled after the Phase sequence closes. Not a
numbered phase.

The Channel Activation Milestone is a dedicated operator-
verification pass that runs **after** the architectural phase
sequence is complete. Its job is to take every channel adapter
that has shipped by that point (`aivyx-telegram` from Phase 8,
and whatever other adapters land in Phase 9+) and run each one
end-to-end against its **real** protocol, credentials, and
network — as a single coherent batch rather than as a per-phase
manual smoke test at each adapter's ship time.

**Why batched rather than per-phase:** manual operator smoke
tests each carry their own credential-juggling tax (BotFather
setup, chat_id discovery, env-var hygiene, real-network latency,
flaky CI hooks). Running them once at the end against a full
adapter matrix is cheaper than running them N times during the
phase sequence, and it catches **cross-adapter interaction**
bugs (e.g., a single audit chain written to by both a local turn
and a Telegram turn, `--verify-only` reporting the combined
count) that per-phase tests structurally can't.

**What the milestone contains:**

1. **Phase 8 Task 7** (deferred): real-bot Telegram smoke test —
   BotFather setup, `--channel telegram` launch, two-message
   persistent-memory round trip across a process restart,
   `--verify-only` forensic walk confirming the cross-restart
   audit chain is intact. The six-step runbook was drafted
   during the Phase 8 working session and will be re-scaffolded
   into the milestone doc when it opens.
2. **Any real-protocol smoke test** for channel adapters that
   ship during Phase 9 and later. Each future adapter brings
   its own scripted-transport unit test suite (like Phase 8
   `aivyx-telegram`'s `run_telegram_session_two_chats_persistent
   _e2e`) and **defers** its real-protocol verification to this
   milestone.
3. **Cross-channel regression sweep:** one local turn + one
   Telegram turn + one-of-each-other-adapter turn against the
   **same** persistent audit chain, then `aivyx --verify-only`
   reporting a combined event count. This is the Phase 8 exit
   criterion rewritten to be N-channel rather than Telegram-
   specific.

The milestone is **not** a phase because it ships no code and
revises no architecture — it's a scheduled operator pass that
either passes (all channels live, cross-channel sweep green)
or produces a list of regressions that open as tickets against
the individual adapter crates. It runs when the Phase sequence
is complete enough that operator verification is worth the
setup cost, which is a judgement call to be made at the time.

## Phase 13 — Role-Config Migration (shipped)

**Frozen — see [PHASE_13.md](PHASE_13.md).** Opened and
exited 2026-04-15. Delivered **P9 — Per-Role Full
Capability Declaration** in four tasks: per-role envelope
fields in `aivyx-config` (Task 1, `2c7acfe`), binary
capability assembly rewritten to walk the declared
parent chain with backcompat-floor substitution per
empty level (Task 2, `af89874`), worked-example
`examples/aivyx.toml` demonstrating the inheritance
primitive including the empty-child surprise case
(Task 3, `a19c6e4`), and a `--print-role` debug flag
for operator introspection of effective envelopes
(Task 4, `3e83422`). Phase 12 Task 3 `default role
config file` deferral closed directly. Workspace tests
rolled 453 → 480 (+27). DESIGN.md streak rolls to
thirteen; PRODUCT.md streak begins at one; production-
core `aivyx-core/src/lib.rs` streak re-establishes to
two (first re-established production-core streak since
Phase 10/11 held and Phase 12 broke it).

## Phase 14 — Sub-Agent Role-Switching (shipped)

**Frozen — see [PHASE_14.md](PHASE_14.md).** Opened and
exited 2026-04-16. Delivered the first concrete piece of
**PRODUCT.md P1 — Sub-Agent Mode via Role-Switching** in
four tasks: lift `assemble_role_envelope` from the
binary into `aivyx-channel/src/role_envelope.rs` (Task
1, `96814e7`, closing the first net-new Phase 13
deferral), add a `role.switch` capability scope with a
target-role `QualifierKind` plus `RoleSwitchTool`
registration in the core tool registry (Task 2,
`7364504`), wire sub-session nesting via an inline child
agent constructed by an `OnceLock`-backed factory closure
on `RoleSwitchTool` (Task 3, `74883e2`), and extend
`--print-role` with a mechanical reachable-switch-target
enumerator (Task 4, `91053ec`). Workspace tests rolled
480 → 509 (+29). All three byte-identity streaks held:
DESIGN.md → fourteen consecutive phases, PRODUCT.md →
two, production-core `aivyx-core/src/lib.rs` → three
(the at-risk streak the phase-open doc flagged for
Task 3 survived via an inline factory-closure path that
required zero `lib.rs` edits). The P1.3 "structural
impossibility of escalation" guarantee is pinned by
integration tests against narrowed-caps child snapshots
*and* by the debug-surface enumerator's structural-
impossibility test, which read from the same
`assemble_role_envelope`-produced `CapabilitySet`.

## Phase 15 — Channel-Lib Consolidation (shipped)

**Frozen — see [PHASE_15.md](PHASE_15.md).** Opened
and exited 2026-04-16 as the **first non-product-shape
sub-phase** in project history. Five tasks: open
commit (Task 1, `2d97cfd`), cross-crate integration
test against `examples/aivyx.toml` closing the Phase
13 Task 3 cross-crate half (Task 2, `1cc94d6`),
renderer lift into `crates/aivyx-channel/src/role_
render.rs` picking up the Phase 14 Task 5 optional
cleanup and shrinking the binary 2706 → 2071 (−635)
(Task 3, `8afa00a`), per-tier worked example
`examples/aivyx-semitrusted.toml` +
`semitrusted_example_e2e.rs` closing the Phase 13
Task 3 per-tier-example half with a mechanical
teaching-comment correction caught inside the task
(D4 Rule 1 short-circuits before qualifier rules —
the original prediction that path-qualified
`fs.read:/tmp/notes/**` would survive
`CEILING_SEMITRUSTED` was wrong; the ceiling omits
the `fs.read` *base* entirely) (Task 4, `02d658d`),
and exit freeze (Task 5). Workspace tests rolled
509 → 519 (+10). All three byte-identity streaks
held, all extending: DESIGN.md → fifteen consecutive
phases, PRODUCT.md → three, production-core
`aivyx-core/src/lib.rs` → four (the longest
production-core run in project history, exceeding
the original Phase 10/11 baseline at its re-
establishment point). Phase 13 Task 3's three-part
deferral is now fully closed across Phase 14 Task 1
(lift), Phase 15 Task 2 (cross-crate test), and
Phase 15 Task 4 (per-tier example). Rolling backlog
10 → 8 items at exit. Only net-new deferral is a
~5-line doc-comment rewrite on
`CEILING_SEMITRUSTED`'s ▲-row wording (tagged with
the Phase 13 Task 4 reflexivity investigation as
the natural co-home). The lift pattern from Phase
14 Task 1 is now validated at a second, much larger
case (635 lines vs. 130); "lift private fns from
the binary into the channel lib" is a confirmed
reusable pattern rather than a one-shot trick.

## Phase 16 — Daemon Migration: Protocol Settlement (phase 1 of N) (frozen)

**Frozen — see [PHASE_16.md](PHASE_16.md).** Opened
and closed 2026-04-16 as the first phase of the
**Daemon Migration keystone** (P4 Daemon-Default
Architecture). Delivered: (1) `docs/DAEMON_IPC.md`
— the load-bearing IPC protocol specification
(length-prefixed JSON over Unix domain sockets,
`DaemonLifecycleEvent` as a separate message type,
OS-user auth per P4.4); (2) a PoC daemon server
(`daemon_server.rs`) + PoC client (`daemon_client.rs`)
+ one round-trip integration test proving the protocol
carries one turn end-to-end over a real Unix socket;
(3) three forward-investment helpers (`PROTOCOL_VERSION`
constant, `default_socket_path()`, `render_for_cli()`).
All six Q-block questions resolved: Q1→(a), Q2→(a),
Q3→(a), Q4→(a), Q5→(a), Q6→(b). **All four byte-
identity streaks held** — the open doc's prediction
that the production-core streak would "probably break"
was wrong; every mitigation argument held. Test delta
+14 (519→533). Zero new workspace dependencies.

## Phase 17 — Daemon Migration: Production Hardening (phase 2 of N)

Phase 16 exited with a working PoC and a settled
protocol shape — path (a) from the Phase 16 open
doc's three-way scaffold. Phase 17 converts the PoC
into a production-ready daemon. Likely scope:

- **Daemon lifecycle hardening.** Graceful shutdown
  on signal, crash detection from the frontend,
  multi-connection support, in-flight turn replay
  after restart.
- **Auto-spawn (P4.5).** The frontend detects no
  running daemon, spawns one transparently, and
  connects. Phase 16 deferred this as Q6→(b).
- **`daemon` subcommand + `--daemon` flag in the
  binary.** The Phase 16 PoC lives at the library
  level; Phase 17 wires it into the CLI.
- **LocalChannel regression-test rewrite over IPC.**
  The existing integration tests continue in-process
  in Phase 16; Phase 17 rewrites them to run over
  the daemon IPC boundary.

Phase 17 is where the production-core streak faces
its strongest test — multi-connection dispatch may
require a transport-aware `ChannelContext` method
(Q2 mechanism #1), and auto-spawn may require
lifecycle primitives that the `StreamEvent` enum
cannot carry without a new variant (Q4 mechanism #2).
Phase 17's open doc should name these risks with the
same honesty Phase 16's open doc did, and the same
mitigation-first approach.

**Multi-level sub-agent nesting** remains on the
candidate list as a light follow-up to Phase 14's
deferral but carries no urgency. Any phase that picks
up the Phase 13 Task 4 `CapabilitySet::grants`
reflexivity investigation should also absorb Phase 15's
▲-row doc-comment rewrite — the two items live in the
same file and scheduling them together saves one round
of `aivyx-capability` regression scope.

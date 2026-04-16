# Phase 16 — Daemon Migration: Protocol Settlement (phase 1 of N)

**Status:** Active (opened 2026-04-16). This document will
churn during the phase and freeze at exit under a final Exit
criteria block at the bottom, matching the Phase 7–15
precedent.
**Predecessor:** [PHASE_15.md](PHASE_15.md) (exit commit
`06dfdfd`, hash backfill `c69a11e`)
**Technical contract:** [`../DESIGN.md`](../DESIGN.md)
(Deliverables 1–8, all LOCKED — unchanged since `e0d6437`,
**fifteen phases running** at Phase 16 entry, streak
**genuinely at risk** in Phase 16 and the open doc says so —
see Streaks at risk below)
**Product contract:** [`../PRODUCT.md`](../PRODUCT.md)
(Commitments P1–P12, all LOCKED — **three phases running**
byte-identical at Phase 16 entry. Phase 16 delivers against
**P4 — Daemon-Default Architecture** and the shape of the
delivery is explicitly *"the contract pins that there *is*
an IPC, not what it is"*, so the PRODUCT.md streak is not
at risk by design — the contract's load-bearing deliberate-
silence on protocol choice is what Phase 16 exists to
resolve in prose)
**Production-core `aivyx-core/src/lib.rs` streak:** at **four**
at Phase 16 entry (the longest production-core run in
project history, per Phase 15 exit). **Genuinely at risk
for the first time since Phase 12.** The open doc names
the tasks where the risk lives and the shapes each risk
could take — see Streaks at risk below.

## Goal

Phase 16 is the **first phase of the Daemon Migration
keystone**. It is deliberately not "ship a working daemon
in one phase." The full migration is, per `docs/
PRODUCT_ROADMAP.md`, expected to span "one or more
dedicated phases" — and the decision this phase exists to
make is *how many phases* by settling the load-bearing
protocol shape before any task hardens production code
around a provisional choice.

Three concrete outcomes when Phase 16 closes:

1. **An IPC protocol shape is written down and committed
   to.** The Phase 16 decisions block records the settled
   answers to: transport (Unix domain socket vs. abstract-
   namespace socket vs. named pipe on non-Unix), wire
   format (length-prefixed JSON vs. bincode vs. MessagePack
   vs. Cap'n Proto), framing (length prefix byte order,
   max message size, keepalive shape), auth model (OS-user
   permissions on the socket path per P4.4), and auto-
   spawn detachment (fork-then-setsid vs. double-fork vs.
   systemd-managed vs. "the frontend execs a detached
   daemon binary"). Each answer is load-bearing for every
   subsequent daemon phase, so the Phase 16 Q-block is the
   phase's most durable artifact — the tasks implement
   whatever the Q-block settles, not the other way
   around.

2. **A proof-of-concept daemon process and a proof-of-
   concept `LocalChannel`-as-frontend exist in the tree,
   wired together through one integration test that
   verifies a single "hello-world turn" round-trips over
   the settled IPC.** This is not production-ready daemon
   functionality. It is a mechanical proof that the
   protocol shape the Q-block settled can carry one turn
   end-to-end without the implementation discovering a
   fundamental mismatch with the existing turn-loop. If
   the PoC surfaces a mismatch mid-implementation, the
   mid-phase correction block absorbs it and the
   decisions block gets rewritten.

3. **Phase 17's shape is pinned at Phase 16 exit based on
   what the PoC surfaced.** Phase 17 is likely "daemon +
   LocalChannel production readiness: every existing
   `LocalChannel` integration test rewritten to run over
   the daemon rather than in-process." But the Phase 17
   scope only becomes writeable after the PoC has shown
   which parts of the turn-loop, audit chain, and storage
   lifecycle need reshaping. Phase 16's exit doc writes
   the Phase 17 scaffold in `docs/ROADMAP.md` from
   observed shape, not predicted shape.

The headline framing: Phase 16 is a **protocol-first phase**
whose primary deliverable is a set of written-down decisions
plus a mechanical proof that the decisions compose with the
existing turn-loop. Its secondary deliverable is the
smallest-possible daemon process that validates the
protocol by carrying one turn. Its tertiary deliverable is
a cleanly-scoped Phase 17 that inherits the protocol plus
the PoC plus a punch-list of what the PoC did not cover.

## Why now

Five structural reasons. The first three are direct roadmap
pressure; the last two are about *why not wait one more
phase*, which matters for justifying the streak exposure.

1. **Phase 15 cleared the foundation for exactly this
   phase.** The `render_role_envelope` lift brought the
   binary to 2072 lines (down from 2741 at Phase 14
   exit), the `assemble_role_envelope` lift made the
   envelope walker reachable from outside the channel
   crate, and the cross-crate integration test harness
   at `crates/aivyx-channel/tests/` is the exact shape
   daemon-vs-frontend process-boundary tests need.
   Phase 15 exit explicitly said "Phase 16 opens from
   the cleanest backlog and smallest binary the project
   has seen since Phase 10 exit" — and the drift-
   reversal Phase 15 delivered is most valuable to the
   phase that has to re-cut the binary along a process
   boundary. Waiting another phase means paying interest
   on that cleanup.

2. **P4 has been the named next keystone since Phase
   12.5 product review.** The Phase 12.5 product-shape
   review (2026-04-15) introduced P4 — Daemon-Default
   Architecture as the single largest forward
   commitment, and every phase since has either been
   substrate (Phase 13 role-config migration) or a
   dependent-but-smaller commitment (Phase 14 P1 sub-
   agent delivery, which is a pure in-process primitive
   by design). Four phases have now opened against the
   shadow of "daemon is coming but not yet." Either it
   comes now or the roadmap starts drifting away from
   what operators were told in the product review.

3. **The IPC protocol shape is the single load-bearing
   decision the whole daemon migration depends on, and
   settling it now is cheaper than settling it later.**
   Every subsequent daemon phase (LocalChannel port,
   Telegram port, tool-process IPC per P12, channel SDK
   per P5, mission primitive per P2) consumes the IPC
   shape as a given. A wrong early choice — protobuf
   when bincode is the right answer, or length-prefixed
   JSON when a CBOR-like self-describing binary wins —
   compounds across four or five phases. A phase whose
   explicit deliverable is "write down the decision"
   forces the tradeoff analysis into prose at the point
   where the cost of changing one's mind is one commit.

4. **Phase 16 is *not* the moment to ship a fully-
   production-ready daemon.** The aggressive version of
   Option A at phase planning — "daemon + LocalChannel
   end-to-end, every test passing over IPC in one
   phase" — was rejected at phase-shape selection
   because the streak exposure is already high for
   protocol settlement alone, and adding production
   readiness on top means the Q-block would have to
   settle protocol *plus* every production concern
   (crash recovery, backgrounding semantics, restart
   replay, the new-operator first-run story) in one
   phase. That is the shape that produces seven-task
   phases, two mid-implementation correction blocks,
   and a Q-block with twelve open questions instead of
   six. Phase 16 conservatively scopes to "protocol +
   PoC + clean Phase 17 scaffold" and lets Phase 17 do
   production readiness from a settled baseline.

5. **Phase 16 is *not* the moment to start Mission
   Primitive or Multi-level Sub-Agent Nesting.**
   Mission Primitive (P2) is the second-biggest forward
   item, but its forward commitment explicitly couples
   to Daemon Migration ("missions must survive process
   restarts, which means they sit on top of the daemon
   migration" — `docs/PRODUCT_ROADMAP.md` line 120).
   Shipping mission primitive before daemon migration
   means either building a non-persistent mission
   primitive that daemon migration has to re-invalidate,
   or building missions on a daemon substrate that does
   not yet exist. Neither is good. Multi-level sub-agent
   nesting is tagged low-urgency for the reasons Phase
   14 Task 3 recorded; picking it up in Phase 16 would
   be "solving a problem that hasn't been raised" at
   the direct cost of deferring a problem that has.

## Non-goals

Phase 16 is **protocol settlement plus one-turn PoC** and
nothing else. A non-exhaustive list of things Phase 16
deliberately will not ship, with the forward pointer for
each:

- **No production-ready daemon lifecycle.** Crash
  recovery, in-flight turn replay, graceful shutdown on
  signal, daemon crash detection from a frontend, restart-
  replay of an interrupted turn — all of these are
  Phase 17-or-later concerns. The Phase 16 PoC launches a
  daemon, runs one turn, and exits cleanly. Anything
  harder is a scope-drift signal.
- **No Telegram-over-daemon port.** `aivyx-telegram`
  remains in its Phase 8 in-process shape. Telegram port
  is a dedicated phase (probably Phase 18) after
  LocalChannel production readiness lands in Phase 17.
- **No `LocalChannel` regression-test rewrite over IPC.**
  The existing `crates/aivyx-channel/tests/*.rs`
  integration tests continue to run in-process. Exactly
  one new integration test covers the daemon round-trip;
  everything else is Phase 17's job.
- **No tool-process IPC (P12).** P12's tool-as-process
  model is a separate milestone that couples to the
  channel IPC shape but has its own design surface
  (which tools become out-of-process, what the handoff
  boundary looks like, whether the tool binary is a
  separate crate). Deferred to its own phase after
  Daemon Migration stabilizes.
- **No channel SDK extraction (P5).** The SDK milestone
  couples to Daemon Migration — its shape is determined
  by the IPC shape — but the actual SDK crate
  extraction is a later phase. Phase 16 will make
  concrete the IPC shape the SDK will consume; it will
  not publish an SDK crate.
- **No DESIGN.md preventive edits.** The streak-break
  risk (see Streaks at risk below) may require a
  DESIGN.md amendment for D1 (turn-loop deliberately
  single-process) or D3 (`ChannelContext` trait shape).
  If an amendment lands, it lands under the amendment
  process — a single file at `docs/amendments/<date>-
  <slug>.md` plus an inline reference in DESIGN.md.
  Preventive edits "just in case" are forbidden; the
  amendment happens only if an implementation decision
  genuinely requires it.
- **No web UI or HTTP introspection surface.** The
  PRODUCT.md P5 commitment mentions a possible localhost
  web UI as a future frontend, but Phase 16 ships no
  network surface at all. The PoC daemon listens on a
  Unix domain socket (or equivalent) and that is the
  only surface it exposes.
- **No third-party-facing contract.** The Phase 16 IPC
  shape is a Phase 16 decision, not a published SDK
  contract. A future SDK milestone may stabilize it as a
  versioned interface; Phase 16 writes it down clearly
  enough that a future SDK milestone can pick it up, but
  does not commit to stability.
- **No new workspace dependencies *except* whichever one
  the wire-format decision names.** The zero-new-dep
  streak is at elevated risk in Phase 16 because the
  wire-format decision may land on a crate the workspace
  does not currently depend on (`serde_json` is already
  in the tree via `aivyx-config`; `bincode` and `rmp-
  serde` are not). If the Q-block lands on `bincode` or
  `rmp-serde`, the streak breaks and the exit doc
  records it. If it lands on `serde_json` or hand-rolled
  framing, the streak holds. The streak is *allowed to
  break* this phase with a named justification — unlike
  the byte-identity streaks, the zero-new-dep streak
  was never claimed to be indefinitely defensible.

## Entry criteria (all met from Phase 15 exit)

- [x] Phase 15 frozen at exit commit `06dfdfd` + hash
      backfill `c69a11e`. See PHASE_15.md.
- [x] `cargo test --workspace` is **519 green** (verified
      at Phase 15 exit, baseline for Phase 16's delta
      math).
- [x] `cargo clippy --workspace --tests -- -D warnings`
      clean at Phase 15 exit.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to
      `ba9a724`. **Streak at four consecutive phases** —
      the longest production-core run in project history.
- [x] `docs/DESIGN.md` byte-identical to `e0d6437`.
      **Streak at fifteen consecutive phases.**
- [x] `PRODUCT.md` byte-identical to `80189b4`. **Streak
      at three consecutive phases.**
- [x] `crates/aivyx-channel/src/bin/aivyx.rs` at **2072
      lines** (down from 2741 at Phase 14 exit, Phase 15
      delivered the drift reversal). Phase 16's binary
      work will likely *grow* this number back — the
      daemon-mode entry point and the daemon-mode
      dispatch path are both new code. The open doc does
      not set a hard cap; Phase 17 is where the binary
      shape settles.
- [x] `crates/aivyx-channel/tests/` directory exists
      with six integration test files. Phase 16 Task 4
      adds a seventh for the daemon-roundtrip PoC.
- [x] `crates/aivyx-channel/src/role_envelope.rs` and
      `crates/aivyx-channel/src/role_render.rs` exist
      from the Phase 14–15 lift sequence and are
      reachable via `aivyx_channel::assemble_role_
      envelope` / `aivyx_channel::render_role_envelope`
      from outside the channel crate. These lifts are
      load-bearing for Phase 16: the daemon process and
      the frontend process need to see the *same*
      envelope math, and the lifts are what make the
      same math callable from two process contexts.
- [x] Phase 13 Task 3's three-part deferral is fully
      closed (Phase 14 Task 1 lift + Phase 15 Task 2
      cross-crate test + Phase 15 Task 4 per-tier
      example). Phase 16 starts from a rolling backlog
      of 8 items.
- [x] Workspace has **10 crates** at Phase 16 entry
      (`aivyx-audit`, `aivyx-capability`, `aivyx-
      channel`, `aivyx-config`, `aivyx-core`, `aivyx-
      crypto`, `aivyx-llm`, `aivyx-memory`, `aivyx-
      storage`, `aivyx-telegram`). Phase 16 may or may
      not add an eleventh — see Q3 below.

## Streaks at risk

Phase 16 is **the highest streak-exposure phase since the
streak discipline began**. This is the honest inverse of
Phase 15's framing. Phase 15 deliberately said "all three
streaks will extend trivially"; Phase 16 deliberately says
"at least one streak may break, and the open doc names the
shapes the break could take so that when it happens it is
not a surprise."

- **DESIGN.md streak (15 → 16, at risk).** The risk lives
  in D1 (turn-loop deliberately single-process) and D3
  (`ChannelContext` trait shape). D1 describes the turn-
  loop as a single-process state machine that owns the
  LLM client, the tool registry, and the audit chain for
  the duration of a session. A daemon migration that
  moves state ownership into a background process does
  not necessarily contradict D1 — "the turn loop runs in
  *some* process" is the actual invariant D1 pins, and
  which process it runs in is implementation. But the
  *wording* of D1 may read as though it assumes in-
  process, and if a reader-future-me interprets D1 that
  way then an amendment becomes load-bearing. The Q-
  block below (Q1) makes the amendment decision an
  explicit phase-open question. The honest position: **a
  D1 amendment may or may not be required, the phase
  open doc records both possibilities, and the decision
  lands in whichever task first surfaces a concrete
  conflict.** If no conflict surfaces, no amendment lands
  and the streak holds.
  D3 (`ChannelContext` trait) is the other at-risk
  deliverable. Today `ChannelContext` lives in
  `aivyx-core` and describes the frontend's interface to
  the turn loop. The daemon migration reshapes this
  trait's call site — the frontend calls the daemon over
  IPC rather than calling `ChannelContext` methods
  directly — but whether the *trait itself* changes is
  the open question. Q2 below pins it.

- **PRODUCT.md streak (3 → 4, not at risk).** P4's
  forward commitment *explicitly* says "the contract pins
  that there *is* an IPC, not what it is" (PRODUCT.md
  line 313, "What this commitment deliberately does not
  say"). The entire load-bearing deliberate-silence in
  P4 is what Phase 16 exists to resolve in prose; the
  contract does not need to be edited to accommodate the
  resolution, because P4 already forecasts that a future
  phase will settle the silence. PRODUCT.md stays byte-
  identical by construction. If a Phase 16 task surfaces
  pressure to edit PRODUCT.md, that pressure is a scope-
  drift signal: it would mean Phase 16 is delivering a
  product-shape decision the contract does not already
  describe, which would mean either scope creep or an
  honest mistake in P4's drafting.

- **Production-core `aivyx-core/src/lib.rs` streak (4 →
  5, genuinely at risk).** This is the load-bearing
  at-risk streak and the open doc is honest about it.
  Three places the streak could break:

  1. **`ChannelContext` trait edits.** If Q2 resolves
     toward "daemon-aware `ChannelContext` variant" and
     the new variant requires a method or associated
     type, `lib.rs` gets edited and the streak breaks
     at Task 2 or Task 3 (whichever first touches the
     daemon-side dispatch code).
  2. **`StreamEvent` variant additions.** The daemon
     PoC may need a new `StreamEvent` variant to carry
     daemon→frontend lifecycle events (e.g.,
     `DaemonReady`, `IpcDisconnect`). Adding a variant
     to an enum in `lib.rs` is a strict streak break.
     Mitigation: *do not* add a variant; carry daemon
     lifecycle over a separate IPC-level message type
     that never crosses the `StreamEvent` boundary.
     Q4 pins this.
  3. **`ToolContext` edits for IPC-identifier carrying.**
     If a tool invocation in daemon mode needs to know
     which IPC connection originated it (for audit
     attribution), `ToolContext` gains a field and the
     streak breaks. Mitigation: resolve the attribution
     question at the session layer rather than the tool
     layer. Q5 pins this.

  **The open doc's honest position: I expect the streak
  to break in Phase 16.** The most likely breaker is #2
  (a `StreamEvent` variant for daemon lifecycle) and
  the mitigation argument against it is good but not
  airtight. If the streak does break, the exit doc
  records which task broke it, why the mitigation
  failed, and re-baselines the streak at the new
  `lib.rs` commit.

- **Zero-new-dep streak (at risk).** The wire-format
  decision (Q3) may land on `bincode` or `rmp-serde` or
  `ciborium`, none of which are currently workspace
  dependencies. If Q3 lands on `serde_json` (already a
  transitive dep via the config crate) or on hand-
  rolled length-prefixed framing, the streak holds. The
  initial lean on Q3 is toward hand-rolled length-
  prefixed JSON frames for Phase 16, on the grounds
  that (a) the shape of a Phase 16 "one turn round-trip"
  PoC does not need a compact binary format, (b) JSON
  is easy to inspect during PoC debugging, and (c) the
  wire format can be upgraded in Phase 17 without
  breaking the Phase 16 decisions on transport/framing
  /auth. If the Phase 16 PoC surfaces a concrete
  ergonomics problem that a binary format solves, the
  decision gets revisited in a mid-phase correction
  block. The zero-new-dep streak has lower discipline
  weight than the byte-identity streaks — it is allowed
  to break with a named justification, unlike the byte-
  identity streaks.

The four-streak risk profile makes one explicit phase-
open prediction: **Phase 16 will extend DESIGN.md and
PRODUCT.md, will probably break production-core, and may
break zero-new-dep.** The exit doc records which of
those predictions held and which did not. If DESIGN.md
breaks as well, that is a categorical scope-drift event
and the exit doc has to explain it specifically.

## Open questions (pinned at phase open unless marked otherwise)

Phase 16's Q-block is the phase's most load-bearing
artifact. Unlike Phase 14 (six Qs mostly about
implementation detail) and Phase 15 (five Qs mostly about
test placement and lift mechanics), Phase 16's Qs settle
the *architecture* that Phase 17, 18, and every
subsequent daemon phase inherits. Each Q below has a
pinned initial lean, and the Q-block resolves in Task 2
(the protocol-design task) and Task 3 (the PoC daemon
task) by task close. A Q that does not resolve at task
close is a scope-drift signal.

### Q1 — Does Phase 16 require a DESIGN.md amendment for D1 (turn-loop deliberately single-process)?

D1 describes the turn-loop as a state machine owning the
LLM client, tool registry, and audit chain "for the
duration of a session." A session today runs in a single
process from `main()` to exit. The daemon migration
splits this: the turn-loop state machine lives in the
daemon's address space; the frontend process speaks to it
over IPC. D1's invariant (the turn loop is a well-defined
state machine with clear ownership of those three
resources) is unchanged, but D1's *illustrative* wording
may read as though in-process is load-bearing.

Three candidate resolutions:

- **(a)** No amendment. D1's key-commitments bullets are
  the contract; the code blocks are illustrative (per
  Phase 7's "DESIGN.md code blocks are illustrative, not
  byte-exact" rule). The daemon migration shifts the
  process boundary without touching the state-machine
  shape, so D1's contract is intact. Streak holds.
- **(b)** Amendment under `docs/amendments/2026-04-
  17-turn-loop-process-boundary.md` that explicitly
  clarifies D1 applies "per turn-loop invocation,
  regardless of whether the invocation runs in the
  frontend or daemon process," with a pointer to this
  phase's decisions block for the actual shape. Streak
  breaks but under the legitimate amendment process.
- **(c)** Amendment *plus* a D1 wording edit in
  DESIGN.md proper, referencing the amendment inline.
  Most invasive; only justified if (a) or (b) leaves a
  reader-future-me genuinely unable to reconcile D1
  with the daemon architecture.

Initial lean: **(a)**. D1's current wording does not
actually say "in-process"; it says "a state machine
owning the LLM client, tool registry, and audit chain
for the duration of a session." A session can live in
the daemon's address space as straightforwardly as it
can live in the frontend's. The amendment becomes load-
bearing only if a Phase 16 task discovers a concrete
reader-confusion point in D1 that the amendment is the
cleanest fix for. Pinned at phase open; Task 2 revisits
if anything surfaces.

### Q2 — Does `ChannelContext` change shape for daemon mode, or does the daemon-vs-frontend split live above `ChannelContext`?

`ChannelContext` (in `aivyx-core/src/lib.rs`) is today
the trait the frontend implements and the turn loop
consumes. Three candidate daemon-mode shapes:

- **(a) `ChannelContext` is unchanged.** The daemon-
  mode dispatch path in `crates/aivyx-channel/src/bin/
  aivyx.rs` constructs a `DaemonChannelContext` that
  implements the existing trait, forwarding each method
  over IPC. The turn loop does not know it's talking to
  a remote frontend. Production-core streak holds.
- **(b) `ChannelContext` gains a daemon-aware method or
  associated type.** For example, an
  `is_remote() -> bool` hook, or an `IpcConnectionId`
  associated type. Production-core streak breaks
  immediately.
- **(c) A sibling trait `DaemonChannelContext` is
  introduced in the channel lib (not `aivyx-core`) and
  the daemon-mode dispatch path uses it instead of
  `ChannelContext`.** Production-core streak holds
  (the change is in `aivyx-channel`, not `aivyx-core`).
  The turn loop would need a generic parameter over
  both traits — unclear whether the existing turn-loop
  shape can express that without `aivyx-core` edits.

Initial lean: **(a)**. The frontend/daemon split is
fundamentally a *transport concern*, not a trait-shape
concern. `ChannelContext`'s methods already describe
the frontend-to-turn-loop call surface at exactly the
right abstraction level; forwarding each method over
IPC is a mechanical exercise inside the daemon's
dispatch path. The production-core streak is preserved
by construction. Pinned at phase open; Task 3 (the PoC
daemon) is where this is first exercised.

### Q3 — What wire format does the IPC protocol use?

Four candidates:

- **(a) Hand-rolled length-prefixed JSON frames.** Four-
  byte big-endian length prefix + JSON payload + no
  trailer. Uses `serde_json` (already in tree). Easy to
  inspect with `nc -U` or a hex dump during PoC
  debugging. Zero-new-dep streak holds. Wire format is
  the least efficient of the four but also the least
  risk of accidental schema drift because every change
  is human-readable.
- **(b) `bincode` with length-prefixed framing.** Binary
  format, compact, fast to encode/decode. Adds
  `bincode` as a workspace dep. Schema changes are not
  self-describing; a client and daemon built at
  different schema versions will read each other's
  messages incorrectly. Streak breaks.
- **(c) `rmp-serde` (MessagePack) with length-prefixed
  framing.** Binary, compact, self-describing
  (field tags preserved). Adds `rmp-serde`. Streak
  breaks. Stronger schema-evolution story than bincode,
  weaker than JSON.
- **(d) Cap'n Proto or Protocol Buffers.** Adds a
  significant compile-time dependency (proto generator
  or capnp crate). Out of scope for a PoC phase.

Initial lean: **(a)**, hand-rolled length-prefixed JSON.
Rationale: Phase 16 is a protocol-*settlement* phase,
not a throughput-optimization phase. The Phase 16 PoC
runs exactly one turn in one integration test; the
overhead of JSON versus bincode is irrelevant at this
scale. Debuggability is genuinely valuable (a PoC
failure at the wire level should be inspectable with a
hex dump, not a decoder). The zero-new-dep streak
holds. The wire format can be upgraded in Phase 17 if
throughput becomes a pressure point — the transport
and framing decisions are the durable ones, and those
survive a wire-format swap. Pinned at phase open; Task
2 is where this gets hardened into code.

### Q4 — How does daemon lifecycle reach the frontend without breaking the `StreamEvent` enum?

The daemon process needs to signal the frontend about
lifecycle events: daemon-started, daemon-shutting-down,
daemon-crashed, IPC-disconnected. Three candidate shapes:

- **(a) A separate IPC message type for lifecycle,
  never crossed into `StreamEvent`.** The frontend's IPC
  client has two receive paths: one for `StreamEvent`s
  (which pass up to the render layer unchanged) and one
  for `DaemonLifecycleEvent`s (which the frontend
  handles locally — e.g., by printing a message and
  exiting). `aivyx-core/src/lib.rs` is not edited.
  Production-core streak holds.
- **(b) A new `StreamEvent::DaemonLifecycle(...)`
  variant.** Simpler dispatch (one message type, one
  receive path) but strictly breaks the production-core
  streak.
- **(c) Lifecycle is signaled by IPC socket close
  semantics only.** A graceful daemon shutdown closes
  the socket with a clean EOF; an ungraceful shutdown
  closes with a reset. No explicit lifecycle messages
  at all. Minimalist; may not carry enough information
  for a good operator experience.

Initial lean: **(a)**. The argument for (a) is the
load-bearing mitigation against the production-core
streak break I predicted above. If (a) works
structurally (i.e., the frontend's IPC receive loop
can demux two message types without `aivyx-core`
edits), the streak can be mitigated. If it does not
work structurally, (b) is the honest alternative and
the streak breaks at Task 3. (c) is too minimal — the
daemon needs to at least tell the frontend *why* it is
shutting down so operator error messages make sense.
Pinned at phase open; Task 3 (the PoC daemon) is where
the structural feasibility of (a) gets tested.

### Q5 — How does tool invocation carry audit attribution across the daemon boundary?

Today a tool invocation in `aivyx-core`'s `ToolContext`
has access to the `ChannelPlatform` and `TurnId` for
audit attribution. In daemon mode, the tool runs inside
the daemon's address space (per P4.2 "daemon holds live
agent state") but the attribution question is: does the
tool need to know which *IPC connection* originated the
turn, for audit attribution purposes?

- **(a) No.** Tools are attributed to the daemon process
  and the `TurnId`. The audit chain records which
  channel initiated the turn as part of the
  `TurnStarted` event (it already does today — Phase 11
  added `ChannelPlatform` to `TurnStarted`), so no new
  field is needed on `ToolContext`. Production-core
  streak holds.
- **(b) Yes.** `ToolContext` gains an
  `ipc_connection_id: Option<IpcConnectionId>` field so
  tools can differentiate "I was called over IPC from
  frontend X" versus "I was called in-process from
  frontend Y." Production-core streak breaks.
- **(c) Deferred.** The Phase 16 PoC has exactly one
  frontend and one daemon connection, so the question
  is not exercised. Record the question in the exit
  doc's deferrals block and revisit in Phase 17 when
  multiple concurrent frontend connections become
  possible.

Initial lean: **(a)**. Channel attribution is already a
`TurnStarted`-level concern, not a `ToolContext`-level
concern — the tool should not care whether its caller
is in-process or remote, because the tool's contract is
"do work within the envelope the turn declares." If a
future use case surfaces (e.g., a tool that needs to
stream progress updates back to the originating
connection specifically, bypassing the turn-loop), that
is a forward-commitment signal, not a Phase 16 signal.
Pinned at phase open; Task 3 confirms the PoC does not
need attribution plumbing.

### Q6 — Does the PoC daemon auto-spawn on first frontend launch, or is it spawned explicitly for the test?

P4.5 ("auto-spawn is invisible in the common case")
commits to operator-transparent auto-spawn in the long
run. The Phase 16 PoC has two candidate shapes:

- **(a) Auto-spawn is in scope for Phase 16.** The PoC
  integration test launches the frontend binary only;
  the frontend detects no running daemon, spawns one,
  and connects. The test verifies the full auto-spawn
  round trip including detachment. Most ambitious;
  highest chance of a mid-phase correction block
  because fork-then-setsid / double-fork semantics are
  subtle and OS-specific. Highest operator-visible
  value delivered by Phase 16.
- **(b) Auto-spawn is deferred to Phase 17.** The PoC
  integration test spawns the daemon explicitly as a
  subprocess, waits for the socket to appear, and then
  launches the frontend against the known-running
  daemon. The frontend's auto-spawn code path is
  stubbed out for the PoC (or gated behind a config
  flag). Less ambitious; clean scope; Phase 17 owns
  the auto-spawn story alongside production readiness.
- **(c) Auto-spawn is written in Phase 16 but the
  integration test uses explicit spawn.** The code is
  landed and unit-tested against a mockable fork
  abstraction; the integration test bypasses it. Hybrid
  — gets the Phase 16 investment into the tree without
  staking the integration test on fork semantics.

Initial lean: **(b)**. Phase 16 is a protocol-
settlement phase, not a lifecycle-management phase.
Auto-spawn is a lifecycle concern whose right home is
the phase that hardens daemon lifecycle generally
(Phase 17). The Phase 16 PoC should fail loudly with
"no daemon running, spawn one explicitly" if the test
invocation does not provide one, because that failure
mode is the most honest representation of where Phase
16 is actually drawing the line. Pinned at phase open;
Task 3 will have explicit-spawn integration-test
scaffolding.

## Draft task breakdown

Five tasks, same cadence as Phases 11–15. Task 4 is the
working-session slot; Task 5 is exit freeze.

### Task 1 — Open commit (this document)

The phase's first commit is this doc itself, the
`docs/README.md` row flip from "no row" to "Active", the
`docs/ROADMAP.md` Phase 16 scaffold replacement (Phase
15's exit-doc text gets replaced with a Phase 16 Active
entry plus a Phase 17 shape-TBD scaffold below it), and
Task 1 entries in the task list. No code, no tests, no
test-delta requirement. Same shape as the Phase 14/15
open commits (`33230df` / `2d97cfd`).

**Acceptance:**

- `docs/PHASE_16.md` exists with the structure of this
  document (goal, why now, non-goals, entry criteria,
  streaks at risk, Q1–Q6, draft task breakdown,
  decisions at phase open).
- `docs/README.md` phase-status table has a Phase 16
  row marked Active.
- `docs/ROADMAP.md` Phase 16 entry replaces the "shape
  TBD" placeholder with a Phase 16 description and a
  Phase 17 scaffold below.
- Commit message: `docs(phase-16): open — Daemon
  Migration protocol settlement phase 1 of N`.

### Task 2 — IPC protocol specification

**Delivers:** a written-down, committed-to IPC protocol
shape that Phase 17 and every subsequent daemon phase
inherits as a given.

**Cut:** create `docs/DAEMON_IPC.md` — a cross-phase
reference doc (like `docs/ADAPTER_PATTERN.md`) that
specifies:

- **Transport:** Unix domain socket at
  `$XDG_RUNTIME_DIR/aivyx/daemon.sock` (fallback to
  `$HOME/.local/share/aivyx/daemon.sock` if
  `XDG_RUNTIME_DIR` is unset). Socket file created with
  mode 0600 per P4.4.
- **Wire format:** per Q3's resolution (initial lean
  hand-rolled length-prefixed JSON).
- **Framing:** 4-byte big-endian length prefix + JSON
  payload. Max message size 16 MiB (operator-overridable
  in a later phase; Phase 16 pins the default).
- **Message types:** `FrontendRequest` (frontend→daemon,
  e.g., `StartSession`, `SubmitInput`, `CancelTurn`),
  `DaemonResponse` (daemon→frontend, carrying
  `StreamEvent`s or acknowledgments),
  `DaemonLifecycleEvent` (daemon→frontend, carrying
  startup/shutdown signals per Q4).
- **Auth model:** OS user ownership per P4.4. The daemon
  rejects any IPC connection whose peer UID does not
  match the daemon's UID. Verification is via `SO_PEERCRED`
  on Linux (tests for other platforms deferred to their
  respective porting phases).
- **Error model:** every `FrontendRequest` gets exactly
  one terminal `DaemonResponse` (success or error) plus
  zero or more intermediate `StreamEvent` frames.

Also: create the smallest-possible implementation crate
or module that parses the above shape. Q3 lands this in
`crates/aivyx-channel/src/daemon_ipc.rs` if the wire
format resolves to JSON (no new crate needed). If Q3
lands on a binary format, a new `crates/aivyx-ipc/` crate
may be appropriate; this is an open subquestion of Q3.

**Test count target:** +4 (parsing round-trip for each of
`FrontendRequest` / `DaemonResponse` /
`DaemonLifecycleEvent`, plus one max-message-size
boundary test).

**Acceptance:**

- `docs/DAEMON_IPC.md` exists with all six sections
  above filled in with decisions, not open questions.
- The Q3/Q4 resolutions are recorded in this document's
  decisions block with pointers to `DAEMON_IPC.md`.
- A Rust module or crate exists that can parse the IPC
  message types round-trip, with at least 4 tests.
- `cargo test --workspace` green; delta ≥ +4.
- `cargo clippy --workspace --tests -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` (streak holds)
  *or* a documented amendment lands at `docs/amendments/
  <date>-<slug>.md` with an inline reference in
  DESIGN.md.
- PRODUCT.md byte-identical to `80189b4` (streak holds).
- Production-core `aivyx-core/src/lib.rs` — **predicted
  to hold** after Task 2 because Task 2 is
  specification work plus a parsing module, neither of
  which should touch `aivyx-core`. If it does break
  here, that is a scope-drift signal (the parsing
  module belongs in `aivyx-channel`, not core).

### Task 3 — PoC daemon process + PoC frontend dispatch path

**Delivers:** a minimal daemon that listens on the IPC
socket, accepts one connection, runs one turn, and
exits cleanly; and a `LocalChannel` dispatch-path
branch that connects to the daemon rather than running
the turn in-process.

**Cut:** add a `daemon` subcommand to `aivyx.rs` (e.g.,
`aivyx daemon run` starts the daemon process), and a
`--daemon` flag on the frontend that routes turns over
IPC instead of in-process. The daemon:

- Reads the role config, opens redb, initializes the
  audit chain.
- Binds the IPC socket at the path from Task 2.
- Accepts one connection, reads one `StartSession`
  message, creates a session owned by the daemon
  process, receives one `SubmitInput` message, runs
  the turn through the existing `run_session`
  machinery, streams `StreamEvent`s back to the
  frontend over the IPC connection, sends a terminal
  `DaemonResponse::SessionComplete`, closes the
  connection, and exits.

The frontend dispatch-path branch:

- When invoked with `--daemon`, connects to the IPC
  socket, sends a `StartSession` message, reads the
  first stdin line, sends it as `SubmitInput`, reads
  streaming `StreamEvent`s from the daemon and renders
  them through the existing `render_stream_event`
  (which Phase 3 shipped and Phase 15 did not touch),
  waits for the terminal `DaemonResponse`, closes
  the connection, and exits.

**Test count target:** +1 — a single integration test
at `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs`
that spawns the daemon process, runs one turn over IPC,
and asserts the streaming output matches the in-
process baseline for the same input. The "one test"
budget is deliberate: Phase 16 is proving the protocol
works end-to-end, not covering every edge case.

**Acceptance:**

- `crates/aivyx-channel/src/bin/aivyx.rs` gains a
  `daemon` subcommand and a `--daemon` flag on the
  default subcommand.
- `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs`
  exists with at least one test that passes.
- `cargo test --workspace` green; delta ≥ +5 across
  Tasks 2 + 3 combined.
- Q1/Q2/Q4/Q5/Q6 all resolved and recorded in this
  document's decisions block (Q3 was already resolved
  by Task 2).
- Three byte-identity streaks held *or* their breaks
  documented with honest "this is where it broke"
  reasoning in the mid-task correction block.

### Task 4 — Working-session slot

Reserved for whatever mid-implementation correction
Phase 16 surfaces. The Q-block predicts several places
a correction could land:

- **Q1 (DESIGN.md amendment).** If Task 2 or Task 3
  surfaces a concrete D1 reader-confusion point, Task
  4 absorbs the amendment drafting.
- **Q3 (wire format correction).** If the hand-rolled
  JSON approach hits an ergonomics wall in Task 3
  (e.g., serializing `Scope` qualifiers is surprisingly
  ugly), Task 4 can re-open the Q3 decision.
- **Q6 (auto-spawn ambition).** If Task 3 lands
  explicit-spawn cleanly and there is task budget left,
  Task 4 can consume the auto-spawn stub work as a
  Phase-16-internal deferral pickup (leaving
  production-readiness auto-spawn to Phase 17).
- **Mid-implementation correction.** If Tasks 2 or 3
  surface a wrong assumption that needs documenting,
  Task 4 absorbs it — same slot shape as Phase 14 Task
  3's Option B → Option F3 pivot and Phase 15 Task 4's
  teaching-comment correction.

None of these is pre-committed. Task 4 opens with a
review of what Tasks 2–3 surfaced and either consumes
one of these candidates, consumes something that came
up during implementation, or is skipped.

### Task 5 — Exit freeze

**What lands:** same shape as Phase 11–15 exit freezes:

- Ship records for Tasks 1–3 (and Task 4 if used)
  written into this document under their respective
  blocks.
- Decisions block recording how Q1–Q6 resolved.
- Phase 16 deferrals block: the eight rolling items
  inheriting from Phase 15 exit plus whatever net-new
  Phase 16 surfaces. Target: rolling backlog at exit
  ≤ 10 items (Phase 16 is a keystone-opening phase so
  some growth is expected).
- Final Exit criteria checklist, green-checkmarked
  line by line.
- `docs/README.md` phase-status row flipped from
  Active to Frozen.
- `docs/ROADMAP.md` Phase 16 entry replaced with a
  refined Phase 17 scaffold. Phase 17's shape is
  determined by what Phase 16 surfaced — likely
  "daemon + LocalChannel production readiness with
  all existing LocalChannel integration tests
  rewritten to run over IPC," but the call is made
  at Phase 16 exit based on observed Phase 16
  outcomes.
- Exit commit under `docs(phase-16): exit freeze …`
  + hash backfill commit matching the Phase 11–15
  recipe.
- A **prediction-versus-reality block** at the end
  of the exit doc, explicitly comparing the open
  doc's streak-risk predictions against what actually
  happened. Phase 16 is the first phase where the
  open doc names streak breaks as expected, and the
  exit doc should pay that forecast the respect of
  explicitly reckoning with it.

**Acceptance:**

- All Task 1–3 (and Task 4 if used) ship records and
  the decisions block are in this document.
- `cargo test --workspace` green at exit. Test delta
  across the full phase **≥ +5** against the 519-test
  entry baseline. Phase 16's target is smaller than
  Phase 14/15's because Phase 16 is protocol-design
  heavy — most of the phase's value is in prose
  decisions, not in test multiplication.
- `cargo clippy --workspace --tests -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` **or** an
  amendment file at `docs/amendments/2026-04-<dd>-
  <slug>.md` with an inline reference in DESIGN.md.
  Streak either extends to sixteen consecutive phases
  *or* breaks with a documented reason and re-
  baselines at the amendment commit.
- PRODUCT.md byte-identical to `80189b4`. Streak
  extends to **four consecutive phases**. Phase 16's
  P4 delivery is explicitly within the
  "deliberately-does-not-say" space P4 already
  describes, so PRODUCT.md should not need editing.
- `crates/aivyx-core/src/lib.rs` — **may or may not
  break**. If it holds, streak extends to **five**
  and the exit doc records which Q-resolution
  mechanics shielded it. If it breaks, the exit doc
  records which task broke it, why the mitigation
  argument failed, and re-baselines the streak at
  the new lib.rs commit.
- Zero-new-dep streak — **may or may not break**.
  If Q3 resolved to (a) the streak holds. If Q3
  resolved to (b) or (c) the streak breaks and the
  exit doc names the crate and the justification.
- Binary line count at exit: no hard cap. The
  daemon subcommand and the daemon dispatch path
  will grow the binary. Phase 17 is where the
  binary shape resettles after daemon functionality
  is production-hardened.
- `docs/README.md` phase-status table reflects
  exit commit hash (backfilled in a separate
  commit).
- `docs/ROADMAP.md` Phase 16 entry replaced with a
  Phase 17 scaffold refined by Phase 16 outcomes.
- `docs/PRODUCT_ROADMAP.md` Daemon Migration
  milestone updated to reflect the landed Phase 16
  shape (protocol settled, PoC in tree, Phase 17
  scoped).
- Phase 14 rolling deferral "Multi-level sub-agent
  nesting" still open; Phase 13 Task 4
  `CapabilitySet::grants` reflexivity still open;
  Phase 15 net-new ▲-row doc-comment rewrite still
  open. None of these is picked up in Phase 16.
- **Prediction-versus-reality block** recorded,
  explicitly reckoning with the open doc's
  forecasts.

## Decisions made at phase open

Recorded here so the phase's intent is legible at a
glance:

1. **Phase 16 is Daemon Migration phase 1 of N, not
   "ship a daemon."** The full migration spans multiple
   phases; Phase 16 settles the protocol and proves it
   carries one turn. Any pressure to expand scope
   toward production readiness is scope drift and
   routes to Phase 17.
2. **The Q-block is the phase's primary deliverable.**
   Unlike Phase 14 (implementation-heavy, Q-block was
   scaffolding) and Phase 15 (hygiene-heavy, Q-block
   was minor), Phase 16's Q-block resolves architecture
   that every subsequent daemon phase inherits. The
   exit doc's decisions block is the phase's durable
   value; the code is the mechanical proof that the
   decisions compose with the existing turn loop.
3. **Streak breaks are named upfront as possibilities.**
   The open doc explicitly predicts that the
   production-core streak may break, names the three
   mechanisms by which it could break, and names the
   Q-resolutions that mitigate each mechanism. This is
   the first phase where "streak break" is acknowledged
   in the open doc as a legitimate outcome rather than
   a failure mode. If the streak does break, the exit
   doc records the break honestly and re-baselines;
   the honesty is the point.
4. **The conservative scope is load-bearing, not
   timid.** The aggressive shape ("daemon + LocalChannel
   end-to-end in one phase") was rejected at phase-
   shape selection because the Q-block alone is big
   enough to justify a phase on its own. A conservative
   Phase 16 produces a Phase 17 that starts from a
   settled protocol baseline; an aggressive Phase 16
   produces a Phase 17 that inherits a protocol that
   was decided under scope pressure. The first
   outcome is strictly better for the migration arc,
   even though it costs one extra phase.
5. **PRODUCT.md's "deliberately does not say" clause is
   the load-bearing protection against the PRODUCT.md
   streak break.** P4's forward commitment explicitly
   forecasts that a future phase will settle the IPC
   shape; Phase 16 is that phase. The PRODUCT.md
   streak survives by construction, and if a Phase 16
   task surfaces pressure to edit PRODUCT.md, that
   pressure is either scope drift or a drafting
   mistake in P4 that the amendment process absorbs.
6. **Phase 16 does not pick up any rolling deferral.**
   The `CapabilitySet::grants` reflexivity investigation
   and the ▲-row doc-comment rewrite could compose
   cleanly with a daemon phase in principle (both
   items touch `aivyx-capability`, which the daemon
   migration exercises), but Phase 16 has enough on
   its plate with the Q-block alone. Deferred to
   whichever future phase meaningfully touches
   `aivyx-capability`.

## Decisions made during implementation

### Task 2 — Q3 resolution: hand-rolled length-prefixed JSON (option a)

**Resolved:** option **(a)**, hand-rolled length-prefixed JSON
frames using `serde_json` (already in workspace). See
[`docs/DAEMON_IPC.md`](DAEMON_IPC.md) for the full specification.

**Rationale:** Phase 16 is a protocol-settlement phase, not a
throughput-optimization phase. The PoC runs exactly one turn;
JSON's overhead is irrelevant at this scale. Debuggability (hex
dump, `socat`) is genuinely valuable for PoC failure diagnosis.
The wire format can be upgraded in Phase 17 without touching the
transport/framing/auth decisions. Zero-new-dep streak holds —
`serde_json` was promoted from dev-dep to prod dep in
`aivyx-channel`, but it was already a workspace dependency.

### Task 2 — Q4 resolution: separate lifecycle message type (option a)

**Resolved:** option **(a)**, `DaemonLifecycleEvent` is a separate
`#[serde(tag = "type")]` enum from `DaemonMessage`. The frontend's
IPC receive loop demuxes on the `"type"` discriminator field into
turn-loop traffic (`DaemonMessage`) vs. lifecycle signals
(`DaemonLifecycleEvent`). A `DaemonEnvelope` union type is provided
for frontends that want a single `decode_frame` call site.

**Streak impact:** `aivyx-core/src/lib.rs` is untouched. The
`StreamEvent` enum gains no variant. Production-core streak holds
through Task 2 as predicted.

### Task 2 — implementation shape

The IPC parsing module landed at `crates/aivyx-channel/src/
daemon_ipc.rs` (not a new crate). `serde_json` promoted from
dev-dep to prod dep in `aivyx-channel/Cargo.toml`; `serde` added
as prod dep (both already workspace deps). Module is `pub` from
`aivyx_channel::daemon_ipc` so the PoC daemon (Task 3) and future
phases can import the types.

**Test delta:** +8 (3 round-trip tests for the three message
envelopes, 1 oversized-encode rejection, 1 oversized-decode
rejection, 1 incomplete-buffer test, 1 `DaemonEnvelope` demux
test, 1 `StreamEventPayload` all-variant round-trip). Target
was ≥ +4; delivered +8.

**Workspace test count:** 527 (entry baseline 519, delta +8).

### Task 3 — Q1 resolution: no DESIGN.md amendment (option a)

**Resolved:** option **(a)**. D1's key-commitments bullets describe
the turn loop as a state machine owning the LLM client, tool
registry, and audit chain "for the duration of a session." The
daemon migration shifts the process boundary — the turn loop now
runs inside the daemon's address space — without changing the
state-machine shape. D1's wording does not say "in-process"; it
says "a state machine owning" those resources. The
`IpcChannelBridge` in `daemon_server.rs` implements `ChannelContext`
by forwarding `StreamEvent`s over IPC, proving the turn loop
does not know it's talking to a remote frontend. **DESIGN.md
streak extends to sixteen consecutive phases.**

### Task 3 — Q2 resolution: ChannelContext unchanged (option a)

**Resolved:** option **(a)**. The daemon constructs an
`IpcChannelBridge<C>` that wraps any existing `ChannelContext` and
implements the `ChannelContext` trait by serializing each
`stream_event` call into a `DaemonMessage::StreamEvent` IPC frame.
The turn loop calls the bridge exactly as it would call an
in-process `LocalChannel`. No methods or associated types were
added to `ChannelContext`. **Production-core streak holds.**

### Task 3 — Q5 resolution: no ToolContext changes (option a)

**Resolved:** option **(a)**. Tool invocations are attributed to
the daemon process and the `TurnId`. The `TurnStarted` audit event
already carries `ChannelPlatform` (since Phase 11). No new field
was needed on `ToolContext`. The Phase 16 PoC has exactly one
frontend and one daemon connection, and the PoC's
`FakeStreamingAgent` does not exercise tools. **Production-core
streak holds.**

### Task 3 — Q6 resolution: explicit daemon spawn in test (option b)

**Resolved:** option **(b)**. The integration test spawns the
daemon server on a background tokio task, waits 50ms for the
socket to appear, then connects the client. Auto-spawn is a
lifecycle concern for Phase 17. The Phase 16 PoC exits cleanly
after one turn; the test verifies the full `DaemonReady` →
`SessionStarted` → `StreamEvent` × 2 → `TurnComplete` sequence.

### Task 3 — implementation shape

Three new files in `crates/aivyx-channel/src/`:

- **`daemon_server.rs`** — `run_poc_daemon` function: binds Unix
  socket, sends `DaemonReady`, reads `StartSession` +
  `SubmitInput`, dispatches one turn through `Agent::turn` via an
  `IpcChannelBridge` that forwards `StreamEvent`s over IPC, sends
  `TurnComplete`, exits. The `IpcChannelBridge` is the Q2
  resolution in code: it implements `ChannelContext` by serializing
  to `DaemonMessage::StreamEvent` frames.

- **`daemon_client.rs`** — `run_poc_client` function: connects to
  the socket, reads `DaemonReady`, sends `StartSession` +
  `SubmitInput`, collects `StreamEvent` frames until
  `TurnComplete`, returns a `DaemonTurnResult`.

- **`tests/daemon_roundtrip_e2e.rs`** — integration test with a
  `FakeStreamingAgent` that streams two text chunks. Verifies
  version, session_id, event count, event content, and outcome
  string. Uses a real Unix domain socket in `$TMPDIR`.

**Tokio features added:** `net`, `io-util`, `sync`, `time` in
`aivyx-channel/Cargo.toml` (both prod and dev deps). These are
feature flags on the existing `tokio` workspace dep, not new crate
dependencies.

**Test delta:** +1 (one integration test). Combined phase delta
Tasks 2 + 3: +9 (target was ≥ +5).

**Workspace test count:** 528 (entry baseline 519, delta +9).

**All six Q-block questions are now resolved.** Q1→(a), Q2→(a),
Q3→(a), Q4→(a), Q5→(a), Q6→(b). All resolutions preserve the
production-core streak. The open doc's prediction that the streak
would "probably break" turned out to be wrong — every mitigation
argument held.

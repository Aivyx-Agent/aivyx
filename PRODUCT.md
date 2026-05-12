# Aivyx Product — Phase 12.5 Review

Drafted 2026-04-15 during the post-Phase-12 product-shape review pass.
This document is the **product analogue of [`DESIGN.md`](DESIGN.md)**: the
locked decisions about *what Aivyx is for and what it is not* that every
subsequent phase must respect. `DESIGN.md` answers "how does the agent
work" — `PRODUCT.md` answers "who is the agent for, what does it
commit to do, and where is the line."

The two contracts are siblings, not nested. Editing either one requires
an amendment under `docs/amendments/` per the same process. The current
phase journal lives in `docs/PHASE_N.md`, and the rolling product-shape
entries live in [`docs/PRODUCT_ROADMAP.md`](docs/PRODUCT_ROADMAP.md) (the
product analogue of [`docs/ROADMAP.md`](docs/ROADMAP.md)).

This document was assembled from a four-cluster question-pinning pass
that walked every load-bearing product question surfaced by a
read-only inventory of the post-Phase-12 codebase. The cluster-by-cluster
pinning rationale is in the appendix at the bottom of this file so a
future session can reconstruct the *why* without reading the original
working-session transcript.

---

## The one-line pitch (LOCKED 2026-04-15)

> **Aivyx is a personal autonomous agent platform that runs on your
> hardware, talks to cloud LLMs under your own API key, and never
> compromises privacy or auditability for the sake of a feature.**

Every commitment below either supports this sentence or constrains
what features the platform may grow without contradicting it. Future
phases that find themselves drifting away from this sentence are
required to surface the drift as an amendment, not absorb it silently.

---

## Product Commitment 1 — Single Operator, Single Primary Agent (LOCKED 2026-04-15)

### The Rule

> **One human operator per Aivyx instance. One primary agent the
> operator interacts with. Sub-agents exist as a mode the primary
> agent switches into mid-session — not as separate processes, not as
> parallel sessions, not as independent tool registries.**

### What this commits us to

1. **No multi-tenancy. Ever.** A second human is a second Aivyx
   instance on a different machine (or under a different OS user
   on the same machine). There is no "shared" mode, no "team"
   mode, no tenancy primitive at any layer.

2. **Sub-agents are role-switching, not process-spawning.** When
   the primary agent needs to operate under constrained authority
   (a child task, a sandboxed exploration, a domain-specific
   sub-mission), it switches into a child role per **P7** and
   runs in that role's attenuated capability envelope. When the
   sub-task completes, it switches back. There is exactly one
   physical agent process at any moment.

3. **Capability escalation across role-switch boundaries is
   structurally impossible.** A child role's capability set is
   strictly attenuated from its parent's per the inheritance
   rules in **P7**. The primary agent cannot grant a sub-agent
   authority it doesn't itself hold; the type system enforces
   this, not a runtime check.

4. **One redb store, one audit chain, one capability ceiling.**
   Sub-agent turns are tagged with the role active at the time
   of the turn, the same way Phase 11's role primitive already
   tags ordinary turns. The audit chain is operator-scoped, not
   role-scoped.

### What this commitment deliberately does not say

- **It does not say sub-agents are easy to use today.** The
  role-switching primitive needed to support sub-agent mode is a
  forward commitment; today's foundation has the role primitive
  but not the mid-session switching machinery.
- **It does not say sub-agents are always sequential.** A future
  phase may explore concurrent sub-agent execution within the
  single physical process; that's an implementation choice, not
  a contract change. The contract pins "single physical process,"
  not "single execution pointer."
- **It does not say what happens when a mission outlives the
  operator's session at the keyboard.** That's **P5**'s territory.

---

## Product Commitment 2 — Session Legibility and Two Success Modes (LOCKED 2026-04-15)

### The Rule

> **Every Aivyx session is operator-inspectable in real time and
> after the fact. Sessions complete in one of two shapes: a
> bounded task with a clear start and end, or an open-ended
> mission with operator-approval gates at decision points.**

### What this commits us to

1. **The legibility floor.** The renderer must surface enough of
   the agent's reasoning, tool calls, and outcomes that the
   operator can judge what happened without reading source code.
   This includes a real-time view (during the session) and a
   post-hoc view (`--verify-only` and any future inspection
   surfaces). A feature that obscures the agent's actions from
   the operator is a contract violation, full stop.

2. **Bounded tasks are the default shape.** A session begins
   when the operator initiates an interaction, runs through one
   or more turn-loop iterations, and ends when the goal is
   satisfied or abandoned. This is the shape Aivyx ships today
   and the shape every channel adapter understands.

3. **Open-ended missions are a forward commitment.** A mission
   is a long-running operator intent that may span multiple
   sessions, multiple days, multiple agent restarts, and is
   structured around **operator-approval gates**: the agent runs
   autonomously between gates, halts at each gate, queues the
   gate for the operator to resolve asynchronously, and resumes
   when approval is granted. Missions are the primitive that
   makes "track this codebase's CI for the next week and tell
   me when something breaks" a first-class shape.

4. **Approval gates are operator-controlled, not agent-controlled.**
   The agent identifies decision points where it judges operator
   approval is warranted (or where the role config requires it).
   The operator can also pre-declare conditions that always
   require approval (file writes outside a path, network calls
   to specific hosts, capability scopes the role doesn't hold,
   etc.). The mission lifecycle, the gate-resolution UX, and the
   operator-side queueing are all in **P5**.

### What this commitment deliberately does not say

- **It does not pin the mission primitive's shape.** What a
  mission *is* (a row in redb, a long-lived turn, a tree of
  sub-sessions) is a future-phase decision. The contract
  commits that mission semantics exist; the implementation
  shape is open.
- **It does not say a bounded task and a mission are different
  primitives.** A mission may turn out to be a bounded task
  with a longer lifetime and approval-gate semantics, or it
  may turn out to need its own type. The contract is
  outcome-shaped, not type-shaped.

---

## Product Commitment 3 — Goals and Non-Goals of the Platform (LOCKED 2026-04-15)

The product contract takes an explicit position on what Aivyx is
trying to become and what it explicitly refuses to become. Each
goal below is a positive commitment that constrains future-phase
work toward a target shape; each non-goal is a constraint that
prevents drift away from the target.

### Goals

- **G1 — Web interaction.** `web.fetch` is the floor. Future
  phases extend Aivyx to richer web interaction (page rendering,
  form submission, structured scraping) **gated by role
  allowlists**. A `researcher` role may have a rich web surface;
  a `coder` role may have raw HTTP only. The goal is not "a web
  agent," it is "an agent the operator can configure to interact
  with the web within a role-defined envelope."

- **G2 — Code interaction.** Aivyx is a coding agent in addition
  to being other things. Full CRUD on files via `fs.read` /
  `fs.write`, shell execution via `shell.exec`, and a future
  TUI-level code-editing surface that the agent uses *as a tool*.
  Aivyx does not become an IDE — it interacts with code through
  the same tool family that interacts with everything else.

- **G3 — Memory Reflection (outcome-driven self-improvement).**
  The agent observes outcomes — every `ToolOutcome`, every
  `TurnOutcome`, every mission-level success or failure — and
  uses that history to refine its own behavior over time. Memory
  is *core*, not scratchpad. The substrate (`memory.*`) is the
  storage layer; the reflection layer is a forward commitment
  whose properties are pinned in **P8**.

- **G4 — Sub-agent orchestration.** The primary agent can switch
  into a child role mid-session per **P1** and **P7**, run
  constrained, and switch back. Sub-agents inherit attenuated
  authority by construction. The contract commits to "sub-agents
  exist as a first-class mode" without committing to a specific
  orchestration UX.

- **G5 — Autonomous and scheduled execution.** Missions per **P2**
  may run unattended, between operator interactions, on schedules
  (cron, systemd timers), in response to triggers (webhooks, file
  changes), or on agent-initiated continuations. All such
  execution is attributed to the operator's identity per **P6**
  and audited against the operator's capability ceiling.

- **G6 — Local execution, cloud inference, privacy non-negotiable.**
  Aivyx runs on the operator's hardware. Inference may go to
  cloud LLM providers (Anthropic today; others as the
  `LlmProvider` trait grows). Storage is encrypted at rest with
  an operator-controlled passphrase. Network egress is gated by
  capability scopes. **Privacy and encryption are never
  compromised in the name of a feature** — this is the single
  property that uniquely positions Aivyx, and the one a future
  phase is most likely to be tempted to compromise on. Don't.

- **G7 — Third-party tool SDK.** Tools follow a documented
  contract any third party can implement to add new capabilities
  to Aivyx. The SDK contract guarantees policy integration:
  capability scopes, audit logging, cancellation, role
  allowlists, input schema enforcement. The SDK shape is pinned
  in **P11**; the distribution model is pinned in **P12**.
  Third-party tools are *possible*, but **N3** rules out
  marketplace dynamics — the SDK contract is open, the registry
  is not.

### Non-Goals

- **N1 — No multi-tenancy or shared instances.** Per **P1**.

- **N2 — No hosted-service shape.** Aivyx is not a SaaS, has no
  hosted control plane, has no Aivyx-the-company server in the
  loop. The operator runs the daemon on their own hardware
  under their own OS user, period. Per **P6** and **G6**.

- **N3 — No marketplace dynamics.** Third-party tools are
  possible per **G7**, but Aivyx core does not host a registry,
  curate a directory, run a rating system, or distribute
  third-party tools. Operators install third-party tools the
  way they install any other software — explicitly, knowing
  what they are doing. The model is `git`'s relationship with
  third-party `git-*` subcommands, not Chrome's relationship
  with the Web Store.

- **N4 — Not an IDE.** Per **G2**: Aivyx interacts with code
  via tools. It does not ship LSP integration, syntax
  highlighting beyond what the renderer already does,
  project-awareness beyond what the filesystem gives it, or
  visual diff UI. A future phase may add a TUI-level
  *code-editing surface that the agent uses as a tool*, but
  the surface is for the agent, not for the operator to use as
  an editor.

- **N5 — No inference-side compromises for convenience.** No
  proxying the operator's API key through a hosted service
  "for setup ease." No batching the operator's prompts on
  Aivyx servers "to save money." No phone-home telemetry. No
  cloud-side context store. The operator's API key talks
  directly to the model provider, the operator's data stays
  on the operator's hardware, full stop.

### What G3, G4, G5 commit to that the foundation does not yet ship

The current foundation (Phase 12) ships a working bounded-task
agent on Local CLI and Telegram. **G3 (Memory Reflection),
G4 (sub-agent orchestration as a switching mode), and G5
(autonomous/scheduled execution) all require future-phase work
to deliver.** The contract pins them as forward commitments;
the corresponding phase work is enumerated in
`docs/PRODUCT_ROADMAP.md`.

---

## Product Commitment 4 — Daemon-Default Architecture (LOCKED 2026-04-15)

### The Rule

> **Aivyx runs as a long-running daemon process under the
> operator's OS user. The user-facing CLI commands are frontends
> that connect to the daemon. If no daemon is running when a
> frontend launches, the frontend auto-spawns one and detaches
> it; the daemon survives the frontend exit. There is exactly
> one operational shape.**

### What this commits us to

1. **The daemon is the only execution shape.** Foreground-only
   mode, as it exists today, is replaced. The same binary
   serves as both daemon and frontend depending on how it is
   invoked, and the operator's normal usage (`aivyx`,
   `aivyx --channel telegram`, etc.) routes through frontends
   to a backing daemon.

2. **The daemon holds live agent state.** Active turns, in-flight
   missions, scheduled-execution timers, IPC connections,
   loaded role configs, the open redb handle — all live in the
   daemon's address space and survive frontend disconnections.
   Restarting a frontend does not restart the agent.

3. **The daemon's IPC surface is the single integration point.**
   Channel frontends speak to the daemon over IPC. Third-party
   tool processes (per **P12**) speak to the daemon over the
   same IPC. Future inspection surfaces (a `aivyx status`
   subcommand, a localhost web UI, etc.) all attach via IPC.
   This is one protocol to design, defend, and version.

4. **IPC authentication is OS-level.** The daemon's IPC socket
   is owned by the operator's OS user with mode 0600. There is
   no Aivyx-level password, token, or auth handshake on the
   IPC. Anyone who can read the socket file is by definition
   the operator. Per **P6**.

5. **Auto-spawn is invisible in the common case.** An operator
   running `aivyx` for the first time on a fresh box should not
   need to learn the word "daemon" before getting a working
   agent. The frontend detects no running daemon, spawns one,
   detaches it, and connects — all transparently. The operator
   only thinks about the daemon when they explicitly want to
   manage it (`aivyx daemon stop`, `aivyx daemon status`).

### What this commitment deliberately does not say

- **It does not say what the IPC protocol looks like.** Unix
  domain sockets, named pipes, length-prefixed JSON, msgpack,
  protobuf — all valid implementation choices. The contract
  pins that there *is* an IPC, not what it is.
- **It does not say the daemon has a network surface.** Per
  **P6** and **G6**, the daemon's IPC is local-only. A web UI
  per **P5** is `127.0.0.1:<port>` only, not a service exposed
  to the network.
- **It does not say how the daemon handles crashes.** Crash
  recovery, in-flight turn replay, mission resumption — all
  implementation concerns for the daemon-shipping phase.

### Phase impact

This is the **largest single architectural commitment** in the
product review. The current foundation is process-per-session
shaped, and the daemon migration will reshape the binary, the
channel adapters, the audit chain wiring, and the storage
lifecycle. The migration is enumerated as the first major
entry in `docs/PRODUCT_ROADMAP.md` and is expected to span
one or more dedicated phases.

---

## Product Commitment 5 — Open First-Party Channel Surface (LOCKED 2026-04-15)

### The Rule

> **Aivyx commits to a documented channel adapter SDK. First-party
> channels are whichever ones are worth shipping in core; the
> contract pins the shape, not the count. Third parties can also
> implement the adapter contract.**

### What this commits us to

1. **The adapter SDK is a first-class artifact, not internal
   plumbing.** It is the same kind of contract as the tool SDK
   from **P11**: documented surface, integration guarantees,
   stability commitments deferred until real use. A third-party
   channel author writes a channel against the SDK and gets the
   turn-loop, audit-chain, and capability machinery for free.

2. **First-party channels grow with the platform.** Today: Local
   CLI, Telegram. Plausibly tomorrow: a localhost web UI on
   `127.0.0.1` (made cheap by the daemon IPC from **P4**),
   voice (made plausible by mobile-side audio), an IDE plugin,
   email, Discord. The contract does not pre-commit to any
   specific second-tier channel — it commits to the *shape* of
   how channels are added.

3. **Local-first applies to channel design.** A first-party
   channel may not require a hosted relay, a cloud provider's
   bot infrastructure beyond what an operator-owned key
   permits, or any third-party service the operator hasn't
   explicitly opted into. Telegram is acceptable because the
   operator runs their own bot under their own token; a
   first-party "Aivyx Hosted Web" channel would not be.

4. **Channel adapters are pluggable at daemon startup, not
   compile time.** The daemon discovers available channel
   frontends and registers them. This is consistent with **P4**'s
   daemon shape and **P12**'s tool-as-process model.

### What this commitment deliberately does not say

- **It does not pre-commit to a localhost web UI.** A web UI is
  a *likely first addition* once the daemon ships, because the
  IPC layer makes it cheap, but it is not pinned. The decision
  is made at the phase that opens it, not here.
- **It does not commit to a stable adapter SDK version.** Same
  posture as **P11**: integration guarantees first, stability
  later when real third-party adapters surface real pressure.

---

## Product Commitment 6 — OS-Level Operator Identity (LOCKED 2026-04-15)

### The Rule

> **The operator is the OS user who owns the daemon process and
> the redb store. There is no Aivyx-level identity primitive,
> no Aivyx-level authentication, no Aivyx-level account.**

### What this commits us to

1. **Identity equals OS user, period.** The operator's
   "identity" is whatever `id -u` returns. Aivyx does not
   maintain its own user table, does not issue tokens, does
   not sign audit entries with an Aivyx-internal key. Audit
   chains attribute work to "the OS user the daemon ran
   under," and that's the identity the contract commits to.

2. **The daemon never grows authentication surface.** Frontends
   attaching to the daemon authenticate via Unix file
   permissions on the IPC socket (mode 0600, owned by the
   operator). There is no password handshake, no token
   exchange, no challenge-response. If you can `read(2)` the
   socket, you are the operator.

3. **Triggered runs (cron, systemd, webhooks) inherit OS
   identity.** A scheduled mission launched by `cron` runs
   under the operator's OS user. A webhook-triggered run is
   delivered by a daemon thread already running under the
   operator's OS user. There is no "service account," no
   shared identity, no impersonation.

4. **A leaked passphrase is rotated against the redb store
   alone.** Because Aivyx has no separate identity primitive,
   compromising the passphrase compromises the storage, but
   it does not require revoking an "Aivyx account" or
   re-issuing certificates. Rotation is a passphrase change
   on the redb store, full stop.

### What this commitment deliberately does not say

- **It does not say the operator cannot use Aivyx remotely.**
  Remote use means SSH'ing into the box and talking to the
  local daemon — exactly the way `tmux`, `mosh`, and other
  long-lived per-user daemons work. Aivyx itself never grows
  a network-listening surface; the network surface is `sshd`,
  which is not Aivyx's concern.
- **It does not say multi-device sync is impossible.** A
  future feature may sync Aivyx state between two machines
  the same operator owns, but that sync is a peer-to-peer
  shape between two operator-owned daemons under one OS
  identity per box, not a cloud-side identity primitive.
  Pinning would be a contract amendment.

---

## Product Commitment 7 — Single-Inheritance Role Tree (LOCKED 2026-04-15)

### The Rule

> **Roles form a single-inheritance tree rooted at an implicit
> `default` role. A child role's authority is strictly attenuated
> from its parent's: tool allowlist subset, capability set
> subset, deeper memory namespace, contained filesystem ceiling.
> Multi-parent composition is explicitly out.**

### What this commits us to

1. **Every role declares an optional `parent`.** A role with
   no `parent` field inherits from the implicit `default`
   role. A role with `parent: Some("base")` inherits from
   `base`. The tree is rooted at `default` and has no cycles
   (validated at config load time).

2. **Inheritance is strict attenuation along every dimension.**
   - `tool_allowlist`: child is a subset of parent's.
   - Capability set (per **P9**): child's capability scopes
     are each strictly contained within a parent's scope.
   - `memory_topic_prefix`: child's prefix is a deeper
     namespace under the parent's (parent `coder/` →
     child `coder/refactor/`).
   - `fs_root` (per **P9**): child's root is a path within
     the parent's root.
   - Network allowlist (per **P9**): child's allowed origins
     are a subset of the parent's.
   - System prompt: child may override entirely; this is the
     only dimension that is not attenuated, because a
     prompt-attenuation primitive doesn't exist.

3. **Sub-agent mode (per P1) switches into a child role.** When
   the primary agent enters sub-agent mode, it switches to a
   child of its currently-active role. This makes sub-agent
   capability constraint a property of the type system: the
   child role's declaration cannot exceed the parent's, so the
   sub-agent cannot escalate.

4. **Aivyx core ships zero default roles.** The `default` role
   is implicit (synthesized at config load time if absent). The
   `coder` and `researcher` test fixtures from Phase 11 stay as
   test fixtures only — operators write their own role tree.
   Future phases may grow operator-facing role config samples
   per **N3**'s curation posture, but no role is *shipped* in
   core.

5. **Multi-parent composition is structurally impossible.** The
   diamond problem is closed off at the contract level. A role
   that needs the union of two parents' capabilities should be
   a sibling of both with its own explicit declarations, not
   an attempt to merge them.

### What this commitment deliberately does not say

- **It does not say the role tree is small.** A complex
  Aivyx deployment may have a deep tree of roles for
  different sub-tasks, missions, and contexts. The contract
  does not constrain depth or width.
- **It does not say roles are immutable at runtime.** Per **P8**'s
  reflection commitment, the agent may write to its own role
  tree under capability gating. Today the role config is
  load-time-only; the reflection layer phase will grow runtime
  role mutation.

---

## Product Commitment 8 — Outcome-Driven Audited Reflection (LOCKED 2026-04-15)

### The Rule

> **The agent observes outcomes — every `ToolOutcome`, every
> `TurnOutcome`, every mission-level success or failure — and
> uses that history to refine its own behavior over time. Every
> behavior change is an explicit, audited operation with the
> same standing as a tool call. There is no silent
> self-modification.**

### What this commits us to

1. **The substrate is `memory.*`, unchanged.** Phase 6's
   memory tools (`memory.read`, `memory.write`, `memory.forget`),
   Phase 10's wildcard cross-topic recall, Phase 11's role-prefix
   isolation. This is the storage layer reflection builds on,
   and it is **not replaced** — extended, never replaced.

2. **Outcome history is first-class data the agent can read.**
   Every tool call's outcome, every turn's outcome, every
   mission's success/failure signal is recorded in the audit
   chain *and* is exposed to the agent through a future
   Reflection API. Today the audit chain records this data;
   the future commitment is that the agent can *read* it as
   structured input to its reasoning, not just produce it as
   side effects of its actions.

3. **Reflection writes are capability-secured.** The agent
   needs an explicit capability (e.g., `reflection.write`,
   exact base name TBD by the implementing phase) to make
   any change to its own role config, system prompt, memory
   topics, or other persistent behavior-shaping state. An
   operator who does not want autonomous self-modification
   simply does not grant this scope to any role, and the
   substrate degrades to "outcome history exists but the
   agent cannot act on it."

4. **Every reflection write lands in the audit chain.** A
   reflection-driven role-prompt edit is an audited operation
   the same way a `fs.write` or `memory.write` is. The audit
   chain remains the single source of truth for "what did
   this agent do," and reflection is not a side-channel.

5. **The reflection layer's *shape* is deferred.** The
   contract commits to the *properties* (outcome-driven,
   audit-logged, operator-inspectable, capability-secured)
   without specifying the implementation. Whether reflection
   is one tool, several tools, a separate subsystem, or an
   emergent property of how the role tree is structured — all
   open questions for the phase that ships it.

### What this commitment deliberately does not say

- **It does not say reflection requires an LLM call.** A
  reflection step may be as simple as "if this tool's
  failure rate exceeds N, remove it from the role's
  allowlist" — pure logic, no inference. Or it may require
  the LLM to summarize a session's outcomes and write a new
  prompt. Both shapes are valid.
- **It does not say reflection is autonomous by default.**
  The capability gate from point 3 is the operator's lever:
  reflection is opt-in per role.
- **It does not say reflection is global.** Per **P7**'s
  attenuation rules, a child role's reflection writes are
  scoped to the child's own state, not the parent's.

### Why this commitment is the most differentiating

This is the single property that distinguishes Aivyx from
"Claude with a memory store" and from "AutoGPT with
self-improvement." Aivyx commits to outcome-driven evolution
**and** to keeping every step of that evolution legible to
the operator. The reflection layer phase is expected to be
one of the most consequential in the platform's lifetime,
and pinning the audit-chain invariant *now* prevents a
future implementation from compromising it for ergonomics.

---

## Product Commitment 9 — Per-Role Full Capability Declaration (LOCKED 2026-04-15)

### The Rule

> **Every dimension a role can be attenuated along is declared
> in the role config — tool allowlist, capability scopes,
> filesystem roots, network allowlists, memory namespacing,
> shell cwd ceilings, and any future capability dimension. The
> role config is the single declaration site for everything a
> role can do. The binary's operator-level capability grants
> are removed and replaced with the active role's declarations.**

### What this commits us to

1. **The role config is the single source of truth.** An
   operator reads one file (their `aivyx.toml` or per-role
   files) to know what any role can do. There are no hidden
   capabilities, no operator-level grants that bypass the
   role, no implicit "the binary always allows X."

2. **Every capability dimension has a role-level field.** The
   exact field set grows as the platform grows, but every
   capability the role primitive can attenuate is reachable
   from the role config. Today's PHaseE-12 dimensions:
   `tool_allowlist`, `system_prompt`, `memory_topic_prefix`.
   The expansion adds: `capability_scopes` (full Scope list),
   `fs_root`, `fs_mode` (read-only / read-write), `net_allowlist`
   (URL prefix list), `shell_cwd_root`, and `parent` (per **P7**).

3. **Inheritance applies to every dimension.** Per **P7**'s
   attenuation rules, a child role's value on every dimension
   must be a strict subset of the parent's. This is how
   single-inheritance role composition gets its real teeth —
   the entire capability surface is composable, not just the
   tool list.

4. **The binary's operator-level grants disappear.** Today the
   binary unconditionally grants `memory.read`, `memory.write`,
   `fs.read:<root>/**`, `fs.write:<root>/**`, `net.fetch`,
   `shell.exec:cwd:<root>/**` to the operator's capability
   set. Per this commitment, that surface moves *entirely*
   into the implicit `default` role's declaration. Explicit
   roles attenuate from the `default` role's declaration, not
   from a separate operator-level grant.

5. **Validation runs at role-load time.** Capability
   declarations exceeding the parent's, references to
   undefined tools, filesystem paths outside the operator's
   outer sandbox, network allowlist entries that don't parse,
   inheritance cycles — all fail loud at startup, not at
   first-use. Operators get a clear error before the agent
   starts a turn.

### What this commitment deliberately does not say

- **It does not pin the field names or the exact TOML shape.**
  Those are implementation choices for the migration phase.
  The contract commits to "every capability dimension is
  reachable from the role config," not "the field is called X."
- **It does not say the migration is small.** This is a
  significant reshape of the binary's capability wiring,
  comparable in scale to Phase 11's role primitive itself.
  The migration is enumerated in the product roadmap.
- **It does not say new capability dimensions land in the
  contract.** Adding a new capability base to the role config
  is an additive change. Removing one or changing inheritance
  semantics would be an amendment.

---

## Product Commitment 10 — Substrate-Only Core, Eight Tools Forever (LOCKED 2026-04-15, amended 2026-04-20)

### The Rule

> **Aivyx core ships exactly eight first-party tools forever:
> `fs.read`, `fs.write`, `memory.read`, `memory.write`,
> `memory.forget`, `shell.exec`, `web.fetch`, `web.post`.
> Adding to or removing from this list requires a
> `PRODUCT.md` amendment.**
>
> *See amendment
> [`2026-04-20-substrate-tool-count.md`](docs/amendments/2026-04-20-substrate-tool-count.md)
> — `web.post` added in Phase 37, amendment filed in Phase 38.*

### What this commits us to

1. **The substrate is the operator-discoverable surface.** An
   operator who runs Aivyx for the first time finds these
   eight tools available (subject to role allowlists and trust
   tiers). Every richer capability — git, browser, LSP, code
   search, email, calendar, anything domain-specific — is a
   third-party tool the operator installs explicitly.

2. **The substrate is closed-set.** The contract pins the
   exact list, not "approximately eight" or "the current set
   plus reasonable additions." Pressure to add a ninth tool
   to core is met with "amendment to **P10** required" — and
   the amendment must explain why the new tool is substrate
   rather than third-party. The default answer is "third
   party," and the burden of proof is on the addition.

3. **The substrate principle is "what every Aivyx instance
   needs to bootstrap."** A tool belongs in core if and only
   if Aivyx without it cannot perform basic operator-useful
   work. `fs.*` and `memory.*` and `shell.exec` and
   `web.fetch`, and `web.post` together cover "read, write,
   remember, execute, fetch, post" — the minimal set for an
   agent that does anything
   useful. Anything richer is curated by the operator's role
   declarations and tool installations, not by Aivyx core.

### Substrate vs. Infrastructure vs. Third-Party — the three-tier taxonomy

The eight-tools-forever rule applies only to *substrate
tools* — the operator-facing primitives an operator chooses
when configuring a role. Aivyx is permitted (and expected)
to grow two adjacent tool categories that are **not**
substrate and therefore **not** counted against the cap:

- **Infrastructure tools.** Tools the agent uses to manage
  itself: reflection writes per **P8**, role updates,
  outcome reads, sub-agent spawning per **P1**, mission
  lifecycle operations per **P2**. These are the machinery
  the agent uses to *be* itself, not the machinery the
  operator uses to give the agent capabilities. They are
  not operator-discoverable in the role config the same
  way substrate tools are; they are wired by the platform
  and gated by capabilities (often dedicated capability
  bases like `reflection.write`, `mission.create`, etc.).
  The infrastructure set is allowed to grow as **G3**, **G4**,
  and **G5** require, without amendment.

- **Third-party tools.** Anything else, implemented against
  the SDK contract from **P11**, distributed and run per
  **P12**. Operators install these explicitly. There is no
  central registry per **N3**.

### Why the cap is at exactly the current count

Phase 37 is the moment the substrate set has stabilized at
eight tools that together cover the minimum
viable agent surface. Pinning the cap at the current count
is a way of saying: **the foundation is done growing the
substrate.** Future product phases focus on the daemon
(**P4**), the role-config migration (**P9**), the
reflection layer (**P8**), missions (**P2**), and the
SDK contracts (**P11**, **P12**) — not on cramming more
tools into core.

### What this commitment deliberately does not say

- **It does not say the existing eight tools are frozen in
  shape.** A future phase may extend `web.fetch` to support
  HEAD, may extend `shell.exec` to support a process-group
  kill API, may extend `memory.read` to support new query
  variants. Tool *internals* are not pinned by this
  commitment; only the *count of operator-facing
  substrate tools* is.
- **It does not say infrastructure tools are unconstrained.**
  Each infrastructure tool gets its own capability base, its
  own audit treatment, and its own role-config integration
  per **P9**. They are not a back door for capability
  expansion — they are a separate category for a different
  purpose.

---

## Product Commitment 11 — SDK Contract: Interface + Integration (LOCKED 2026-04-15)

### The Rule

> **Any tool implementing the SDK contract gets the policy
> machinery for free: capability scopes are honored exactly as
> declared, audit logging is automatic, cancellation is
> honored, role allowlists work, input schemas are enforced at
> the planner layer. Stability of the SDK API is deferred —
> the contract commits to "what the SDK does," not yet to
> "the SDK never breaks."**

### What this commits us to

1. **Policy integration is automatic.** A third-party tool
   author writes a tool against the documented `Tool` trait
   (and its supporting types: `ToolContext`, `ToolOutcome`,
   `Scope`, `JsonSchema`), declares a `required_scope`, and
   gets *all* of Aivyx's safety properties as guarantees,
   not as opt-ins. The author cannot accidentally skip
   audit logging, cannot accidentally bypass the capability
   check, cannot accidentally evade cancellation. The SDK
   contract is closed under safety.

2. **The contract is the public API of the relevant crates.**
   The crates that constitute the SDK surface (`aivyx-core`'s
   `Tool` trait and supporting types, `aivyx-capability`'s
   `Scope` machinery, the future tool-process IPC protocol
   from **P12**) are the documented surface. Internal helper
   functions are not part of the contract.

3. **The SDK is the same surface first-party tools use.**
   Per **P12**, first-party substrate tools speak the same
   protocol third-party tools speak — they just skip the IPC
   hop for performance. This means the SDK contract is
   self-testing: every first-party tool is also a valid
   third-party tool by construction. A future phase that
   wants to extract a substrate tool into an out-of-process
   third-party tool can do so without rewriting it.

4. **Stability is deferred until the SDK has stabilized in
   real third-party use.** The contract today commits to
   "policy integration works as documented," not to "the
   trait surface won't change between phases." A future
   amendment will pin a stability window (likely versioned,
   likely with a deprecation policy) once enough third-party
   tools exist to apply real pressure.

### What this commitment deliberately does not say

- **It does not say what language the SDK is in.** Today the
  SDK is Rust because Aivyx is Rust. The IPC protocol from
  **P12** opens the door to non-Rust tools speaking the same
  contract; the language-independent surface becomes part
  of the SDK contract once **P12**'s IPC protocol lands.
- **It does not say the SDK ships with a registry, a
  scaffolding tool, or a starter template.** Those are
  ergonomic affordances a future phase may add; the contract
  pins the policy guarantees, not the developer experience.
- **It does not commit to backwards compatibility.** A
  Phase 13 SDK shape may differ from a Phase 14 SDK shape.
  The commitment to stability is a future amendment.

---

## Product Commitment 12 — Tools as Separate Processes Over Daemon IPC (LOCKED 2026-04-15)

### The Rule

> **Third-party tools run as their own OS processes, communicate
> with the Aivyx daemon over IPC, and are registered at daemon
> startup. First-party substrate tools are a special case: they
> ship in-process for performance, but they speak the same
> protocol third-party tools speak.**

### What this commits us to

1. **Process isolation is the security boundary.** A
   misbehaving third-party tool cannot crash the Aivyx
   daemon, cannot read the daemon's memory, cannot exfiltrate
   the redb passphrase from process address space, cannot
   tamper with audit entries that haven't yet reached disk.
   The OS process boundary is the defense, not the SDK API.

2. **Tool processes inherit OS-level identity from the
   daemon.** A third-party tool process runs as the same OS
   user as the daemon (per **P6**). This means the tool
   process *can*, at the OS level, read files the daemon
   reads — including the redb file, including environment
   variables. The IPC isolation defends against in-memory
   attacks; OS-level isolation is implicit in the
   single-operator trust model. **Operators install
   third-party tools knowing they run under their own OS
   user.** This is the same trust model as installing any
   other software on a personal box.

3. **Cross-language tool authoring is unlocked.** Because
   tools speak a documented IPC protocol rather than a Rust
   trait, third-party tools may be written in any language
   that can speak the protocol. A Python data scientist can
   write an Aivyx tool without learning Rust. The IPC
   protocol becomes the language-independent dimension of
   the SDK from **P11**.

4. **First-party tools special-case in-process for speed.**
   The eight substrate tools from **P10** ship in-process
   in the daemon for latency reasons. They speak the same
   protocol third-party tools speak — the protocol is the
   contract — but they bypass the IPC hop. This means a
   substrate tool can be extracted into a separate process
   later (or vice versa) without rewriting it. The
   in-process / out-of-process distinction is a performance
   optimization, not an architectural divide.

5. **Tool registration happens at daemon startup, not at
   compile time.** The daemon discovers available
   third-party tool processes (from a known directory,
   from a config field, or via a discovery protocol),
   verifies their SDK contract version, and registers
   them into the tool catalog. Operators can add or
   remove tools without rebuilding Aivyx.

### What this commitment deliberately does not say

- **It does not say the tool process protocol uses any
  particular wire format.** JSON, msgpack, protobuf,
  Cap'n Proto — all valid implementation choices.
- **It does not say tools cannot be sandboxed further.** A
  future phase may add OS-level sandboxing (seccomp,
  containers, separate UIDs) on top of the IPC isolation.
  The contract pins "process isolation is the floor," not
  "process isolation is the ceiling."
- **It does not say first-party tools may never be
  out-of-process.** A future phase may extract a substrate
  tool into a separate process for fault-isolation reasons
  even at a small latency cost. The contract is symmetric.

---

## Status

This document is the **locked product contract**. Edits require
an amendment under `docs/amendments/` per the same process as
[`DESIGN.md`](DESIGN.md). The technical contract in
`DESIGN.md` and this product contract are siblings: technical
amendments do not require product amendments and vice versa,
but a product amendment that requires a technical change
necessarily requires an accompanying technical amendment.

For the rolling product-shape entries (the product analogue
of `docs/ROADMAP.md`), see
[`docs/PRODUCT_ROADMAP.md`](docs/PRODUCT_ROADMAP.md). For the
current phase journal, see the active `docs/PHASE_N.md` listed
in [`docs/README.md`](docs/README.md). For the contract-vs-roadmap
split explanation, see [`docs/README.md`](docs/README.md).

---

## Appendix — How this contract was assembled

This document is the output of a **product-shape review pass**
conducted at the Phase 12 / Phase 13 boundary on 2026-04-15.
The pass ran in three stages:

1. **Stage 1 — Read-only inventory.** A targeted walk across
   five load-bearing seams of the post-Phase-12 codebase: the
   binary entry surface, the channel adapters, the role
   primitive and capability ceilings, the seven shipped tools,
   and the audit chain. The walk produced a factual,
   non-prescriptive inventory of "what Aivyx looks like from
   outside today."

2. **Stage 2 — Question pinning.** From the inventory, twelve
   load-bearing product questions were surfaced — places where
   the implementation had silently made a decision but never
   written it down, or where no decision had been made at all.
   The questions were grouped into four clusters and pinned
   one cluster at a time, with labelled options (A/B/C and
   variants) and a recommendation for each. The pinning
   followed the same rhythm as Phase 11 Q1–Q6 and Phase 12
   Q1–Q6 in the technical layer.

3. **Stage 3 — This document.** Each pinned answer became one
   numbered Product Commitment (P1 through P12), drafted in
   the same shape as `DESIGN.md`'s deliverables: a one-rule
   north star, a "what this commits us to" list, a "what this
   deliberately does not say" list, and (where applicable) a
   "phase impact" note for commitments that require future
   work to deliver.

### The four clusters

- **Cluster 1 — Identity & intent (P1, P2, P3).** Who runs
  Aivyx, what counts as success, what is in scope and what
  is explicitly out. The single most reframing cluster: the
  pinned answers expanded the product vision substantially
  beyond what the foundation today implements, taking the
  product from "personal CLI agent with eight tools" to
  "personal autonomous agent platform with daemon, missions,
  reflection, sub-agents, third-party SDK."

- **Cluster 2 — Surfaces & channels (P4, P5, P6).** The
  daemon-default architecture (the largest single
  architectural commitment), the open channel adapter
  surface, OS-level operator identity. The cluster's
  centrepiece is **P4**'s daemon commitment, which reshapes
  the binary's lifecycle and unlocks **G5**'s autonomous
  execution and **P2**'s mission primitive.

- **Cluster 3 — Capability and policy model (P7, P8, P9).**
  Single-inheritance role tree, outcome-driven audited
  reflection, per-role full capability declaration. Together
  these form a closed evolution loop: reflection observes
  outcomes → updates a role's declaration within the
  parent's attenuation envelope → next session runs under
  the updated role. This cluster contains **P8**, the single
  most differentiating commitment in the contract.

- **Cluster 4 — Tool suite & SDK (P10, P11, P12).** The
  substrate-only core principle, the SDK contract's
  integration guarantees, and the tool-as-process model.
  Together these define the platform extension surface.
  **P10** locks the current shape; **P11** and **P12**
  open the door for third-party expansion without compromising
  the privacy or audit invariants.

### Internal-consistency check

After all twelve commitments were pinned, an explicit
internal-consistency check surfaced four implications that
needed to be written into the contract explicitly to avoid
drift:

1. **One IPC layer carries channels and tools both.** **P4**'s
   daemon IPC, **P5**'s channel adapter SDK, and **P12**'s
   tool process protocol all share one IPC surface. This
   makes the daemon-shipping phase load-bearing for the
   entire extension story; the IPC protocol designed there
   becomes the contract third parties speak.

2. **Reflection implies runtime role mutation.** **P8**'s
   reflection layer + **P9**'s per-role capability declaration
   together mean role configs become *mutable at runtime*,
   not just at startup. The reflection-layer phase will need
   to grow runtime role mutation under capability gating.

3. **Substrate vs. infrastructure tool taxonomy.** **P10**'s
   seven-tools cap applies only to *substrate* tools.
   Infrastructure tools (the machinery for **G3**, **G4**,
   **G5** to work) are a separate category and may grow
   without amendment. This distinction is written explicitly
   into **P10** and into the appendix to prevent a future
   phase from tripping over it.

4. **Tool processes are trusted at the OS-user level,
   sandboxed at the daemon address-space level.** **P12**'s
   process isolation defends the daemon's memory; OS-level
   isolation is implicit in the single-operator model from
   **P1** and **P6**. A third-party tool can read what the
   operator can read; it cannot tamper with the daemon's
   in-memory state or audit chain. This is written into
   **P12** explicitly.

No contradictions surfaced during the consistency check.
The four implications above are consequences of the pinned
commitments, not unresolved questions.

### Forward commitments enumerated

The following Product Commitments require future-phase work
to deliver. The corresponding entries in
`docs/PRODUCT_ROADMAP.md` give one-paragraph intents per
expected phase:

- **P4 — Daemon-default architecture.** Major reshape, expected
  to span one or more dedicated phases.
- **P9 — Per-role full capability declaration.** Migration of
  the binary's operator-level grants into the role config.
- **P5 — Open channel adapter SDK.** Likely couples to the
  daemon-shipping phase.
- **P12 — Tool process IPC protocol.** Likely couples to the
  daemon-shipping phase.
- **P11 — SDK documentation surface.** Likely a docs-and-
  examples phase after the in-tree SDK has stabilized.
- **P2 — Mission primitive.** Couples to the daemon and to
  the role-config migration.
- **P1 — Sub-agent role-switching machinery.** Couples to
  **P9**'s role-config migration.
- **G3 / P8 — Reflection layer.** Couples to outcome history
  exposure and runtime role mutation.

The commitments that **do not** require new phase work
because the foundation already supports them:

- **P3 — Goals and non-goals.** Vision document.
- **P6 — OS-level operator identity.** Already true; the
  commitment is not introducing it.
- **P7 — Single-inheritance role tree.** Phase 11's role
  primitive is the substrate; **P9**'s migration adds the
  inheritance layer.
- **P10 — Substrate-only core.** True at eight tools
  (Amendment A5, Phase 38).

---

## Delivery Status (as of Phase 50 exit, 2026-05-12)

A traceability surface mapping each product commitment to its
implementation state after 49 phases. The commitment text
above is unchanged (except P10's seven→eight amendment) —
this section records what shipped, what partially shipped,
and what remains forward.

**As of Phase 50 exit, all twelve product commitments are
fully shipped.** The forward-commitment ledger is closed and
P12's "extractable without rewriting" clause is now proven by
the `p12_equivalence.rs` conformance test. Phase 50 also
closed the two Phase 49 bridge deferrals (`ToolEvent` channel
relay; per-call targeted cancellation).

### Fully Delivered

- **P1 — Sub-Agent Role-Switching.** Phase 14 (inline
  sub-session nesting), Phase 33 (multi-level nesting).
  `OnceLock`-backed `RoleSwitchTool` factory closure with
  capability-bounded recursive nesting — each child's
  envelope can only narrow, never widen. Structural
  impossibility of escalation pinned by integration tests
  and `--print-role` reachable-targets enumerator.

- **P2 — Session Legibility and Two Success Modes.** Phases
  21, 23, 28, 35. Mission state machine (six states),
  `MissionCreateTool`, gate creation from
  `TurnOutcome::Escalated` (turn loop + daemon + trigger
  path), `ResolveGate` handler with turn resumption on
  approval, `mission.list`/`mission.status` read-only tools,
  gate rendering (CLI + Telegram).

- **P4 — Daemon-Default Architecture.** Phases 16–20 (five-
  phase migration). Protocol settlement, production hardening,
  REPL wiring, multi-connection + Telegram port, daemon
  management. Auto-spawn, graceful shutdown, PID file,
  `daemon run`/`status`/`stop` subcommands, `--no-daemon`
  flag. The daemon is the sole execution shape in production.

- **P6 — OS-Level Operator Identity.** Always true by
  construction. The daemon's IPC socket is mode `0600`, owned
  by the operator's effective UID. No Aivyx-level identity.

- **P7 — Single-Inheritance Role Tree.** Phase 11 (role
  primitive) + Phase 13 (config migration). `parent_role` in
  TOML config, strict attenuation along every dimension,
  validated at config-load time.

- **P8 — Outcome-Driven Audited Reflection.** Phases 28–30.
  Audit introspection via `turn.history` (Phase 28),
  reflection loop via `reflection.propose`/`.apply`
  (Phase 29), runtime role mutation via `role.update` +
  planner factory integration (Phase 30). Full
  observe→propose→approve→apply cycle operational for
  both memory writes and runtime role-config changes.

- **P9 — Per-Role Full Capability Declaration.** Phase 13.
  `capability_scopes` parsed via `Scope::parse` at config-load
  time. Four-role worked example in `examples/aivyx.toml`.
  `--print-role` debug flag for operator introspection.

- **P10 — Substrate-Only Core, Eight Tools Forever.** Always
  true. The eight substrate tools (`fs.read`, `fs.write`,
  `memory.read`, `memory.write`, `memory.forget`, `shell.exec`,
  `web.fetch`, `web.post`) are the closed set. `web.post`
  added in Phase 37, amendment A5 filed in Phase 38.
  Infrastructure tools and third-party MCP tools are separate
  categories.

- **P5 — Open First-Party Channel Surface.** Phase 48.
  `docs/CHANNEL_SDK.md` is the v0 third-party contract;
  `examples/python-channel/` is the worked reference (Python
  3, stdlib only, 15-test conformance suite). The substrate
  was always present (Phase 16 daemon IPC, Phase 19
  multi-connection, Phase 39 Web UI proves it works for
  out-of-Rust adapters) — Phase 48 *published* it.

- **P11 — SDK Contract: Interface + Integration.** Phase 48
  (channel half) + Phase 49 (tool half). Each SDK's
  `docs/*_SDK.md` declares integration guarantees (capability
  gating, audit logging, cancellation) as stable while
  explicitly deferring API stability per the original P11
  posture ("stability deferred until real third-party use").

- **P12 — Tools as Separate Processes Over Daemon IPC.**
  Phase 49 foundation + Phase 50 closeout. `aivyx-tool` crate
  (12th workspace member, A4 addendum); `ToolProcessBridge`
  spawns child processes via tokio with `kill_on_drop`;
  `ToolProxy` implements `aivyx_core::Tool` over a length-
  prefixed JSON protocol on the child's stdin/stdout;
  operator declares tools in `[[tool_process]]`; scope binding
  at handshake with operator-narrowing overrides;
  `docs/TOOL_SDK.md` is the v0 contract;
  `examples/python-tool/` ships the worked wordcount
  reference. Phase 50 closed the "extractable without
  rewriting" clause: `run_tool_as_subprocess<T: Tool>`
  wraps any `aivyx_core::Tool` impl as a subprocess,
  proven equivalent to in-process execute() by the
  `p12_equivalence.rs` conformance test against
  `FsReadTool`. Phase 50 also wired the two foundation-
  phase deferrals: `ToolEvent` frames now relay onto the
  channel (`Status`/`OutputChunk` → `StreamEvent`), and
  cancellation is targeted via `CancelInvocation { call_id }`
  using a caller-supplied id. **Fully delivered.**

### Partially Delivered

- **P3 — Goals and Non-Goals.** Vision document — partially
  realized through implementation:
  - **G1 (Web interaction):** `web.fetch` shipped (Phase 12),
    `web.post` (POST/PUT/PATCH/DELETE) shipped (Phase 37),
    binary body support and redirect following shipped
    (Phase 37). Rich web interaction (page rendering, form
    submission) not yet started.
  - **G2 (Code interaction):** Shipped. `fs.read`, `fs.write`,
    `shell.exec` all operational with role gating.
  - **G3 (Memory Reflection):** Shipped (Phases 28–30).
    Memory substrate, reflection loop, runtime role mutation.
  - **G4 (Sub-agent orchestration):** Shipped (Phases 14, 33).
    Multi-level role-switching with capability attenuation.
  - **G5 (Autonomous/scheduled execution):** Shipped (Phases
    26–27). Cron schedules, webhooks, file watchers,
    trigger unification, mission wrapping.
  - **G6 (Local execution, privacy):** Shipped (Phase 34).
    Ollama first-class support with health check.
  - **G7 (Third-party tool SDK):** Shipped (Phases 48, 49).
    `docs/CHANNEL_SDK.md` and `docs/TOOL_SDK.md` document the
    v0 contracts; `examples/python-channel/` and
    `examples/python-tool/` are the worked references. MCP
    integration remains as a parallel adapter (Phases 23–24, 32).

### Forward (Not Yet Started)

*(Empty as of Phase 49 exit. The Phase 49 deferral list —
first-party in-process protocol unification, per-tool
sandboxing on top of process isolation — lives in the
phase journal and the rolling roadmap deferral backlog,
not in the product contract's Forward column.)*

### Forward Commitment Candidates — Status Update

The following were identified at the Phase 21 boundary as
candidates. Several have since been delivered:

- **MCP Client Integration.** Delivered (Phases 23–24, 32).
  `aivyx-mcp` crate with stdio and SSE transports,
  `McpServerBridge` + `McpToolProxy`, `mcp.call` capability
  base, `[[mcp_server]]` TOML config, `--mcp-server` and
  `--mcp-sse` CLI flags.

- **Multi-Provider LLM Support.** Delivered (Phases 25, 34).
  OpenAI-compatible `LlmProvider` adapter, `ProviderKind`
  enum (`Anthropic`, `OpenAi`, `Ollama`), `--provider` CLI
  flag. Phase 34 added first-class Ollama support with
  optional API key, conditional `stream_options`, health
  check, and worked example.

- **Scheduled Execution.** Delivered (Phases 26–27). Cron
  schedules, webhook triggers (localhost-only), file-change
  watchers, `TriggerDispatch` unification, opt-in mission
  wrapping.

- **Web UI Channel.** Delivered (Phases 39, 47). Phase 39
  shipped the chat surface (localhost-only `127.0.0.1:7843`,
  `FrontendType::Web`, embedded HTML/CSS/JS, WebSocket bridge
  to the daemon IPC). Phase 47 extended it into a full
  operator inspection surface (mission dashboard, audit
  viewer with verify-chain banner, sessions list) by adding
  the `Query`/`QueryResponse` IPC envelope.

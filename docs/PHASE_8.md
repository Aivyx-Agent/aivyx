# Phase 8 — Ecosystem: Telegram adapter

**Status:** Active (opened 2026-04-14)
**Predecessor:** [PHASE_7.md](PHASE_7.md) (exit commit `8164317`, frozen at `c854cbf`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **seven phases running**)

This document is the **working journal** for Phase 8. It will churn.
At phase exit it is frozen under the same convention as
[`PHASE_7.md`](PHASE_7.md) — no edits except through commits tagged
`docs(phase-8):`.

## Goal

Land the **second concrete `ChannelContext` adapter** — Telegram —
and close the gap between "the trust-tier ladder is specified in
D4" and "the trust-tier ladder has been exercised end-to-end by a
real untrusted network channel." Since Phase 3 shipped
`LocalChannel` as the `TrustTier::Local` proof-of-concept, the
trait and the tier table have been waiting for their next real
impl. Phase 8 is where that impl lands and where the pattern for
*any* future `Trusted` / `Untrusted` adapter (Matrix, Discord,
desktop GUI) gets hammered out.

By phase exit, the `aivyx` CLI can be invoked with a Telegram bot
token and will:

- long-poll the Telegram Bot API for new messages,
- route each incoming message through the existing turn loop with
  a `TelegramChannel: ChannelContext` whose `trust_tier()` returns
  `TrustTier::Untrusted`,
- attenuate the per-turn capability set according to the tier
  table before the turn loop's scope check sees the request,
- persist every turn through the same audit chain Phase 7 made
  durable, so a bad message over the untrusted channel is still in
  the log tomorrow,
- partition memory per-chat via a new `session:<chat_id>` scope
  qualifier (or whatever Q2 below resolves to), so two Telegram
  users talking to the same bot don't see each other's memory.

The headline outcome: `aivyx --channel telegram` (or however Q4
resolves) is a real deployable agent surface, not a developer-laptop
toy. This is the first phase where the project has a credible
answer to *"how do I talk to this agent from my phone,"* and the
first phase where D2's `ChannelContext` abstraction has earned its
keep by *running on something other than a terminal*.

This is the phase that **closes the loop on D2**. Phase 2 shipped
the audit trait, Phase 3 shipped `LocalChannel`, Phase 7 made the
audit durable — and Phase 8 is the phase that runs the whole stack
over an untrusted network channel for the first time. Every one of
Phase 7's hardening items is load-bearing here: persistent audit
(so an attacker over the network can't erase their traces), memory
size caps (so "remember X" 10 000 times doesn't eat the store),
`chmod 0600` (so other users on a shared host can't read the
session state), and interactive passphrase prompting (so the
adapter process can't be the one that decides how to get the
master key — it has to be handed one at startup).

## Non-goals

- **Not a second non-local adapter.** Matrix, Discord, Slack, and
  Email are explicitly **Phase 9+**. Phase 8 ships *one* adapter
  because the point is to find the shape of "an adapter" by
  building one, not to ship four half-finished ones. Once the
  Telegram adapter is green, the second adapter will either (a)
  fall out mechanically from whatever trait-level refactor Phase 8
  revealed was necessary, in which case it takes a week, or (b)
  require a different shape because Telegram's long-poll model is
  not representative, in which case Phase 8's retrospective will
  name what to fix before Phase 9 opens.
- **Not a desktop GUI.** The *other* end of the trust-tier ladder —
  `TrustTier::Trusted` via a local GUI process — is a different
  shape entirely (IPC/socket, local auth, no network). Phase 9+.
- **Not federation, multi-agent, or agent-to-agent messaging.**
  D7 describes this as a future direction; no phase has opened
  the can yet and Phase 8 is not the one.
- **Not a webhook-mode deployment story.** Phase 8 ships long-poll
  first because it's simpler for dev boxes and CI. Webhook mode is
  strictly easier *once* long-poll is green (the transport flips,
  the turn-loop interaction doesn't). See Q4 below — the leaning
  is "design the adapter so webhook is a drop-in replacement, but
  ship long-poll in Phase 8."
- **Not rich media.** Text in, text out. Photo / sticker / voice
  handling is a separate surface that needs its own tool family
  (`telegram.download_media`?) and its own audit event shape. Phase
  8 is the phase that earns the right to add those; it is not the
  phase that adds them.
- **Not inline queries, callback buttons, or BotFather
  configuration.** All post-MVP Telegram features. Phase 8 ships
  the plain "user sends message → bot replies" loop and nothing
  beyond it.
- **Not a multi-tenant hosting story.** One aivyx process per bot
  token. Running 100 bots from one process is a different shape
  and needs its own lock-story across the redb file.
- **Not a D2 amendment.** Phase 8 builds on the existing
  `ChannelContext` trait. If Telegram reveals a contract bug in
  the trait, that's a Phase 8 open question (Q6 below) — not a
  foregone conclusion. Keep the empty-diff streak alive for an
  eighth phase running *if and only if* the contract actually
  fits. The Phase 6 Q5 rule (**honesty over streak preservation**)
  still applies.

## Entry criteria (all met from Phase 7 exit)

- [x] Persistent audit chain survives restarts. `aivyx-audit::
      PersistentAuditLog` is wired into the binary and every turn
      lands on disk under `KeyDomain::Audit`. *(Phase 7 Tasks 1–3.)*
- [x] `aivyx --verify-only` cold-path forensic entry point exists
      and runs without `ANTHROPIC_API_KEY`. *(Phase 7 Task 3.)*
- [x] `rpassword::prompt_password` interactive passphrase lit up
      behind a private closure seam; binary picks `Env` /
      `InteractivePrompt` / bail via `io::stdin().is_terminal()`.
      *(Phase 7 Task 4.)*
- [x] `MemoryWriteTool` refuses writes above
      `AIVYX_MEMORY_MAX_PER_TOPIC` with a typed `Failed` outcome.
      *(Phase 7 Task 5.)*
- [x] Store file + salt sidecar `chmod 0600` on cold start.
      *(Phase 7 Task 6.)*
- [x] Seven-phase DESIGN.md empty-diff streak on entry — D1–D8
      unchanged since `e0d6437`.
- [x] `LocalChannel` exists as a reference `ChannelContext` impl
      and the turn loop routes through it end-to-end. *(Phase 3.)*
- [x] `ChannelPlatform::Telegram` variant already in the enum. The
      tag was added opportunistically in an earlier phase and is
      the first thing Phase 8 gets for free — no `aivyx-core`
      touch needed to name the platform.
- [x] `TrustTier::{Local, Trusted, Untrusted}` ladder is in D4 and
      the `Tool::required_scope(&self, input: &Value)` seam is
      already live, so per-turn capability attenuation doesn't
      need a new trait method.

## Draft task breakdown

This is a *draft*. Phase 6 and 7's breakdowns survived mostly
intact from entry to exit; Phase 2's did not. Tasks are allowed to
reorder and re-scope as we learn.

1. **New `aivyx-telegram` crate.** Standalone crate under
   `crates/aivyx-telegram` with the minimum dep set: `teloxide`
   (or `frankenstein` — see Q5) for the Bot API wrapper, `tokio`
   for async, and a dep on `aivyx-core` for `ChannelContext` +
   `StreamEvent` + `TurnOutcome`. Ship:
   - `TelegramChannel` struct implementing `ChannelContext` with
     `trust_tier() == Untrusted` and `platform() == Telegram`.
   - An async `listen(...)` entry point that long-polls
     `getUpdates`, maps each `Message` to a turn input, invokes
     the turn loop, and sends the reply via `sendMessage`.
   - Unit tests using a mock HTTP transport (no network). Minimum
     coverage: single-message round trip, two concurrent chats,
     cancellation mid-turn.

2. **Per-chat session identity.** Resolve Q2 (see below). Ship
   whatever shape Q2 picks — either one `aivyx` store per chat
   ID, or a shared store with `session:<chat_id>` qualifiers on
   memory and audit events. Land a corresponding integration test
   that proves two chats under the same bot token can't see each
   other's memory.

3. **Scope attenuation at the adapter boundary.** Resolve Q3.
   Wherever the attenuation lives (adapter, trait, helper), Task
   3 wires the Telegram channel's incoming capability set to the
   narrowed version before `run_session` sees it. Includes:
   - A negative test where a `memory.forget` tool call arrives on
     an `Untrusted` channel with an overly broad scope and gets
     rejected at the tier boundary, not at the scope check.
   - Documentation of the attenuation rule in PHASE_8.md, not
     `DESIGN.md` — unless Task 3 reveals that D4's tier table
     needs a missing row.

4. **Binary wiring: `aivyx --channel telegram`.** The existing
   `aivyx-channel` binary learns a new `--channel` flag (or
   whatever Q4 resolves to) that picks between `LocalChannel`
   and `TelegramChannel`. The Telegram branch reads the bot token
   from whatever Q1 picks, constructs a `TelegramChannel`, and
   enters the long-poll loop. The existing stdout/stdin path is
   unchanged when `--channel` is not passed — backwards-compatible.

5. **Turn cancellation across the network.** Resolve Q5 (no, Q5
   is library choice — see Q7). The question: what plays the role
   of `ctrl-C` for a Telegram turn? Candidates: a `/cancel`
   command, a wall-clock timeout, or a per-chat active-turn lock
   that a second `getUpdates` can observe. Ship whatever lands,
   covered by an integration test that starts a turn and cancels
   it via the chosen mechanism before the LLM's first response
   chunk.

6. **Integration test: two chats, one bot, one process.** A
   scripted end-to-end test using a mock `teloxide` transport that
   exercises: bot token handoff, long-poll ingestion, two
   concurrent chats, per-chat memory isolation, the full
   persistent-audit round trip, and the `TrustTier::Untrusted`
   attenuation at the boundary. Same shape as Phase 7 Task 7's
   `audit_persistence_e2e.rs` — local fake transport, scripted
   message injection, deterministic assertions.

7. **Real-bot smoke test (manual).** Run against a real Telegram
   test bot from a dev box: send "remember my favorite color is
   purple", restart, send "what is my favorite color", assert the
   bot recalls correctly via the persistent memory + audit chain.
   This is a Phase 8 exit-criteria deliverable but is not an
   automated test — document the steps in PHASE_8.md's final
   retrospective, not in CI.

8. **Phase 8 exit.** Freeze PHASE_8.md, update README.md and
   ROADMAP.md for Phase 9, refine the Phase 9 entry with whatever
   Phase 8 taught us about the trait-level seams a second adapter
   will need.

## Task 1 — shipped

**Landed:** 2026-04-14. Commit: _pending_.

New crate `crates/aivyx-telegram` is in the workspace. Ships:

- **`TelegramChannel<T: TelegramTransport>`** (crate-private so the
  private `TelegramTransport` bound doesn't leak) implementing
  `ChannelContext`. `platform() == ChannelPlatform::Telegram`,
  `trust_tier() == TrustTier::SemiTrusted` — **not** `Untrusted` as
  the ROADMAP and this doc's Goal section originally said. See the
  correction below.
- **Private `TelegramTransport` trait** with exactly two async
  methods (`get_updates` + `send_message`) adapted to narrow
  `IncomingMessage` / `OutgoingMessage` types. Production impl is a
  `ReqwestTransport` wrapper around `frankenstein::client_reqwest::Bot`
  — wired but not exercised by Task 1 tests on purpose; the whole
  point of the seam is that unit tests run against a `ScriptedTransport`
  test double that never touches HTTP.
- **Buffered streaming model**: `stream_event(Text)` copies the
  `&str` chunk into an owned `String` behind a `Mutex`, and
  `finalize()` drains-and-sends **once**. One turn = one Telegram
  message, unlike `LocalChannel` which flushes per token. Tool-call
  markers and status lines render into the same buffer so the user
  sees a single in-chat message rather than one bubble per tool call.
- **8 unit tests, 0 network**: metadata, session stability,
  cancellation rotation, buffered-send contract, tool-marker
  rendering, empty-turn `"(no reply)"` placeholder, finalize footer
  for each non-Completed outcome, and transport-error propagation
  via a `ScriptedTransport::inject_send_error` knob.

**Validation:** `cargo test --workspace` = 292 passed, 0 failed, 1
ignored (284 → 292, exactly the 8 new telegram tests, no regressions).
`cargo clippy --workspace --all-targets -- -D warnings` clean.

### Trust tier correction: SemiTrusted, not Untrusted

The ROADMAP and this doc's "Goal" section both said Telegram would be
`TrustTier::Untrusted`. That was wrong, and Task 1 is where the
correction lands — per the Phase 7 convention of "fix in the task
ship record, don't retro-edit entry-time doc sections."

The concrete problem: `aivyx-capability::CEILING_UNTRUSTED` grants
exactly `memory.read:scope:public:*` and `audit.read:public`. That is
an anonymous-webhook tier, and it is insufficient for *any* useful
Telegram bot — the bot can't write memory, can't call the LLM, can't
make a network request. `CEILING_SEMITRUSTED` grants `memory.read`,
`memory.write`, `llm.call`, `llm.embed`, `net.fetch`, `net.dns`,
`fs.metadata`, and `config.read`, which is the right minimum for a
bot that can answer questions, persist memory, and do web lookups but
can't write local files. And D4's own text for `SemiTrusted` reads
**"Tier 2 — Authenticated user on a remote channel"**, which is a
literal description of a Telegram chat: the Bot API gives us a stable
`chat_id` + `user_id`, authenticated by Telegram's backend before we
ever see the update.

`Untrusted` stays reserved for its real use cases: unauthenticated
webhook ingest, email with no verified sender, and any adapter where
the channel can't prove *who* is speaking. A Telegram bot can.

This means Q3 (scope attenuation at the channel boundary) now has a
concrete target: attenuate to `CEILING_SEMITRUSTED`, not
`CEILING_UNTRUSTED`. Q3 itself is unchanged — "where does the
attenuation live" is still the same question — but "attenuate to
*what*" now has an answer.

### Other Task 1 sub-decisions

- **Library choice (Q7 resolved):** `frankenstein 0.49` with the
  `client-reqwest` feature. Chosen over `teloxide` because aivyx's
  turn loop IS the dispatcher — a framework that owns its own event
  loop would fight the agent's control flow. Frankenstein exposes
  plain `get_updates` / `send_message` async calls, which is exactly
  the shape the private transport trait wants to adapt.
- **Transport indirection shape:** a *private* trait with two
  methods, not a generic over `frankenstein::AsyncTelegramApi` (the
  ~90-method trait). The narrow surface collapses HTTP + Bot API
  errors to a single `TransportError::Platform(String)` at the seam,
  so the channel's error handling is uniform, and keeps the test
  double trivially small.
- **Buffer model:** `std::sync::Mutex<String>`, not
  `tokio::sync::Mutex<String>`. Every critical section is a
  synchronous `push_str` that never holds the lock across an await.
  `std::sync::Mutex` is the right choice under contention-free
  workloads and matches the `LocalChannel::writer` pattern.
- **`frankenstein 0.49` migration gotcha**: the library flattened
  `Update.message` into a `UpdateContent::Message(Box<Message>)`
  enum variant at some version between 0.30 and 0.49. The
  `ReqwestTransport::get_updates` match arm absorbs this; because
  the private transport trait sits between `frankenstein` and the
  channel, no downstream code in `telegram_channel.rs` had to know.
  This is the first concrete payoff of the transport seam, before
  any "swap transports for tests" motivation.
- **One-chat-per-channel (Task 1 only):** a `TelegramChannel` is
  bound to a single `chat_id` for Task 1. Multi-chat is Q2's
  problem — "one store, many channel instances keyed by chat_id"
  versus "one channel instance with a chat-id qualifier on every
  event" is a Task 2 decision that Task 1 intentionally doesn't
  foreclose.
- **Empty-turn placeholder:** a turn the LLM ended without speaking
  any `Text` chunk (pure-tool turn) produces `"(no reply)"`. The
  Bot API rejects empty `sendMessage` and we don't want the first
  silent tool turn to fail the whole session; dropping "(no reply)"
  into chat is visible graceful degradation.
- **DESIGN.md + aivyx-core empty diff preserved.** Eighth
  consecutive phase deliverable that touches zero bytes of the
  contract documents or the core trait file. `ChannelContext` and
  `ChannelPlatform::Telegram` shipped in Phase 0/3 in exactly the
  shape a second adapter needed, which is the D2 contract earning
  its keep.

## Task 2 — shipped

**Landed:** 2026-04-14. Commit: _pending_. **Resolves Q2.**

Task 2 shipped per-chat memory partitioning. Two `TelegramChannel`
instances sharing one `InMemoryMemory` cannot observe each other's
entries; the `memory.read` / `memory.write` / `memory.forget` tools
derive a session-qualified scope (`memory.read:topic:notes:session:1001`)
that the capability gate enforces, and the substrate stores physical
keys under a reserved `\x01s\x01<session>\x01<topic>` prefix so the
logical topic namespace stays flat for the agent while the storage
is physically isolated.

**Option B: tool-layer namespacing, substrate untouched.** At Task 2
planning time the question was whether to modify the `Memory` trait
(`get_recent(&self, topic, limit, session: Option<&str>)`, ~200
lines across many files) or to do the namespacing at the tool layer
alone. The shipped shape is Option B:

- The `Memory` trait is **unchanged**.
- `RedbMemory` is **unchanged**.
- `InMemoryMemory` is **unchanged**.
- All the namespacing lives in `aivyx-memory::tools` — the logical
  topic the agent passes gets composed with the session into a
  physical storage key inside `execute()`, and the restoration step
  (`entry.topic = topic.clone()` on the way back) prevents the
  physical key from leaking into agent-visible output or the audit
  chain.

This is a strictly narrower change than Option A and it keeps the
`Memory` trait's single-argument shape that Phases 6 and 7 built
on, which made Phase 7's GC and quota work easier to reason about.

**The source of `session`.** A new `ChannelContext::session_partition()
-> Option<String>` method (default `None`) is the per-channel seam.
`TelegramChannel::session_partition()` returns `Some(chat_id.to_string())`;
`LocalChannel` inherits the default `None` (single-partition
behavior, byte-for-byte identical to Phases 6/7). The turn loop
(`agent.rs::run_tool_call`) reads that value and stamps it onto the
tool's JSON input as a reserved `"session"` field *after* the
planner emits the tool call but *before* `required_scope` runs, so
the capability gate sees the session qualifier and memory tool
`execute()` sees it too. The LLM never sees this field — it is not
in any tool's `input_schema`, and `input_schema` is advisory-only at
runtime so the planner can't observe the injection either.

**Reserved-prefix tripwire.** Topics that literally start with
`\x01` are rejected at `required_scope` time regardless of whether
a `session` is present. Without this, an agent with
`memory.read:topic:notes:session:A` could forge a topic like
`\x01s\x01B\x01notes` and side-door into chat B's physical storage
key. See `reserved_prefix_topic_is_denied_even_with_session`.

**Per-topic GC tripwire is now per-session.** Phase 7's per-topic
cap counts entries at the *physical* topic level, which means chat
A filling its `notes` bucket to cap does **not** refuse chat B's
unrelated `notes` writes. Pinned by `per_topic_cap_is_per_session`
in the memory tool tests.

**Tests added:**
- 9 new memory tool unit tests (`aivyx-memory/src/tools.rs`):
  scope derivation with/without session, scope distinctness across
  sessions, empty-session degradation, reserved-prefix denial,
  physical-isolation on write/read, forget isolation, per-session
  cap.
- 1 new telegram test (`aivyx-telegram/src/tests.rs::two_chats_isolated`):
  two `TelegramChannel`s + one shared `InMemoryMemory`, drives the
  full tool surface (write A, write B, read A sees only A, read B
  sees only B). The test contains a faithful inline copy of the
  turn-loop injection so future drift between the two sites would
  fail loudly.

**Validation:** `cargo test --workspace` = 302 passed, 0 failed, 1
ignored (292 → 302, exactly the 10 new tests: 9 memory + 1 telegram,
no regressions). `cargo clippy --workspace --all-targets -- -D warnings`
clean.

### Streak impact: the streak holds at eight — but only just.

The Task 2 plan flagged this as the phase's streak-ender candidate.
The resolution: `DESIGN.md` is still byte-for-byte untouched (git
diff HEAD -- docs/DESIGN.md produces zero output), but
`crates/aivyx-core/src/lib.rs` did get a new default method on
`ChannelContext`. The judgment call is whether adding a default
method to a trait in the core crate counts as breaking the
empty-diff streak.

**The answer is no, for three reasons that matter.**

1. **Every existing `impl ChannelContext` still compiles unchanged.**
   `LocalChannel`, the `NoopChannel` inside `aivyx-memory`'s own
   tests, and every test fake in the workspace get the default
   `fn session_partition(&self) -> Option<String> { None }` for
   free. There is no porting burden on adapters that do not want
   multi-session semantics.
2. **`DESIGN.md` never froze the exact method list of
   `ChannelContext`.** It froze the *shape* — `async_trait`,
   cancellation semantics, finalize/stream_event/reset contract,
   the tier-and-platform metadata pair. None of that moved. The
   new method is an extension point that slots into the existing
   shape, not a contract change.
3. **The alternative was worse for the streak.** Option A
   (modify `Memory`) would have touched the `Memory` trait in
   `aivyx-memory/src/lib.rs` and then forced a D2 amendment
   anyway because the turn loop would have needed a new
   parameter plumbed through. Option B trades one default-method
   addition in `ChannelContext` for *zero* changes to `Memory`,
   `RedbMemory`, or any agent-/channel-facing contract.

So the streak tally stays at eight phases with no `DESIGN.md`
edits, but with a footnote: Phase 8 Task 2 is the first time the
core trait file grew a method since the D2 freeze, and the
judgment that this extends rather than breaks the contract is
documented here so a future phase can reference the precedent
(or challenge it).

### Other Task 2 sub-decisions

- **Reserved-byte namespacing with `\x01s\x01<session>\x01<topic>`**
  instead of a delimiter like `:` or `/`. A printable delimiter is
  forgeable by a malicious agent that controls the logical topic
  string; the reserved `\x01` byte is rejected at
  `topic_uses_reserved_prefix` time, so there is no valid logical
  topic that can masquerade as a physical key. Same idea as the
  SQL injection defense of "don't string-format untrusted
  input into a key space."
- **Audit events emit the *logical* topic, not the physical one.**
  `MemoryReadTool::execute` emits `AuditTag::MemoryAccess` with
  `query_or_key: topic.clone()` where `topic` is the agent's
  original request. If the audit chain ever grew the physical key,
  `verify_from_disk` would have to know about the namespacing
  scheme to round-trip, which would break D1's "audit verifies
  without live substrate" rule.
- **Empty `session` string degrades to `None`.** A channel impl
  that returned `Some("")` would be a channel bug, but the tool
  layer still falls back to unsession-qualified rather than
  emitting a nonsensical `memory.read:topic:notes:session:` scope
  that no capability bundle would ever grant. Pinned by
  `empty_session_is_treated_as_none_for_scope_derivation`.
- **`input_schema` is not updated to advertise `"session"`.** The
  field is turn-loop-injected machinery, not a planner-facing
  parameter. The schema is advisory-only at runtime (no JSON
  schema validation is enforced on tool input), so the agent
  never sees this field and the schema stays clean.

## Open questions

### Q1. Where does the Telegram bot token live?

**Status:** open at phase entry. Must resolve before Task 4.

Three options:

1. **Env var `AIVYX_TELEGRAM_TOKEN`.** Symmetric with
   `AIVYX_PASSPHRASE` / `ANTHROPIC_API_KEY`. Zero new surfaces,
   and the operator already knows the env-var convention. But
   a leaked shell history or `/proc/*/environ` disclosure leaks
   the token, and rotating the token requires restarting the
   process.
2. **A row under `KeyDomain::Secrets` (already exists), bound to
   a well-known key like `b"telegram.bot_token"`.** Encrypted at
   rest, rotatable without restart (via a future
   `telegram.rotate_token` tool), and unified with how the
   passphrase protects every other secret. But adds a "first
   run" flow: the operator has to `aivyx secrets set
   telegram.bot_token <value>` before starting the bot, which
   is a CLI surface Phase 8 doesn't currently have.
3. **A CLI flag `aivyx --telegram-token <value>`.** Same pitfall
   as env vars (shell history) with a second disadvantage
   (process-listing disclosure via `ps auxww`). Dismissed.

**Leaning:** (1) for Phase 8, (2) as a follow-up. The env-var
path gets the bot live fastest and matches the operator's
existing mental model; the secrets-row path is strictly better
but lands the `aivyx secrets set` CLI surface as scope creep.
Ship (1), document the migration path to (2) in the freeze doc.

### Q2. Per-chat session identity — one store per chat, or shared store with qualifiers?

**Status:** **resolved at Task 2 ship (2026-04-14).** Option (2) —
shared store with `session:<chat_id>` qualifiers — landed, with the
qualifier threaded through a tool-layer topic-namespacing scheme
that left `Memory` and `RedbMemory` untouched. See the "Task 2 —
shipped" section above for the full Option B rationale. The
streak-ender warning at the bottom of this question did not fire:
`DESIGN.md` remains byte-for-byte unchanged, and the single
`aivyx-core` edit (a default method added to `ChannelContext`) is
argued to extend the contract rather than break it.

Two shapes:

1. **One `aivyx` store per chat ID.** Mechanically cleanest:
   every chat gets its own `$XDG_DATA_HOME/aivyx/telegram-<chat_id>.redb`,
   every chat has its own master key, every chat has its own
   audit chain. Complete isolation — no "memory leak across
   chats" failure mode is even possible. But: N master keys, N
   audit chains, N `chmod 0600` calls, and passphrase prompting
   at bot startup can't work (the bot needs to open stores for
   chats that haven't yet appeared). Either the bot holds one
   master key that unlocks all per-chat stores (derivation
   symmetric to Phase 7's `SubKey`), or it stores per-chat keys
   under a bootstrap store.
2. **Shared store with `session:<chat_id>` qualifiers on memory
   and audit events.** One store, one passphrase, one audit
   chain — but the audit chain now interleaves events from
   multiple chats, and `memory.read` / `memory.write` events
   carry a `session:<chat_id>` scope alongside the existing
   `topic:<topic>` scope. The `Tool::required_scope` seam
   already supports multi-qualifier scopes (Phase 6 shipped that
   surface explicitly), so Task 2 is adding a second qualifier,
   not inventing a new shape.

**Leaning:** (2). The `Tool::required_scope` seam was designed
exactly for this and Phase 6's `PHASE_6.md:488-497` explicitly
named "session-scoped memory qualifiers" as the exact refinement
that would light up the second qualifier the first time a phase
had a multi-chat model. Phase 8 is that phase. The streak
preservation case is: this is additive at the `Scope::parse` /
tool-input level, not a trait change.

**Counterargument:** the shared-store path interleaves audit
events from multiple chats, which means a bad event from one
chat can wedge the chain for every chat. That's a weaker
isolation story than option (1). Revisit at Task 2 once there's
a concrete audit scenario.

**Streak impact:** if Q2 goes option (2) cleanly, the streak
holds at eight. If option (2) requires a D4 amendment to tier
the qualifiers, or if option (1) requires a D2 amendment to
add per-session state to `ChannelContext`, the streak breaks and
Phase 8 ships its first amendment file.

### Q3. Scope attenuation at the channel boundary — where does it live?

**Status:** open at phase entry. Must resolve before Task 3.

D4's tier table specifies *that* `Untrusted` capabilities are
narrower than `Trusted` capabilities, but not *where* the
narrowing happens. Four options:

1. **At the adapter.** `TelegramChannel::listen` builds the
   narrowed `CapabilitySet` per-turn before calling
   `run_session`. Simplest; the turn loop and tools see a
   pre-attenuated set and the existing scope check handles
   everything else.
2. **At the `ChannelContext` trait.** New method
   `fn attenuate(&self, full_caps: &CapabilitySet) -> CapabilitySet;`
   that every adapter implements. Generalizes option (1) to
   future adapters but is a D2 amendment — the trait grows a
   method.
3. **A standalone `TrustTierPolicy` helper in `aivyx-capability`.**
   `TrustTierPolicy::narrow(tier, full_caps) -> CapabilitySet`
   as a pure function the adapter calls. No trait change, but
   introduces a central policy module that future tiers have to
   update.
4. **Per-tool decision via a new `Tool` trait method.** Each
   tool decides whether it accepts an `Untrusted` invocation.
   Dismissed: too fine-grained, too easy to regress silently.

**Leaning:** (1) or (3). Option (2) is a D2 amendment for a
concern that is *in practice* channel-specific for now; (3) is
the right shape if the attenuation logic starts getting reused,
but if Phase 8 is the only caller it's just overhead. Revisit at
Task 3 once there's working Telegram code.

### Q4. Binary surface — `--channel telegram` flag, subcommand, or separate binary?

**Status:** open at phase entry. Must resolve before Task 4.

Three shapes:

1. **Flag: `aivyx --channel telegram`.** Minimal surface change,
   backwards-compatible (absence of the flag defaults to
   `local`). The flag parser in
   `crates/aivyx-channel/src/bin/aivyx.rs` already handles
   `--verify-only` as a Phase 7 precedent.
2. **Subcommand: `aivyx telegram [args...]`.** More growable but
   invents a subcommand surface the binary doesn't currently
   have. If Phase 9 adds Matrix and Discord, a subcommand tree
   ages better than a `--channel` enum flag.
3. **Separate binary: `crates/aivyx-telegram/src/bin/aivyx-tg.rs`.**
   The `aivyx` command stays identical to Phase 7; a new
   `aivyx-tg` command ships alongside it. Clean separation but
   duplicates the binary wiring (passphrase flow, storage open,
   audit chain, graceful shutdown) across two files.

**Leaning:** (1) for Phase 8, (2) as a Phase 9 refactor when the
second network adapter arrives. Ship the flag, keep the binary
single-entry-point, and let the subcommand refactor happen when
it actually has two things to choose between.

### Q5. Long-poll vs webhook — start with which?

**Status:** open at phase entry. Must resolve before Task 1.

Telegram's Bot API supports both. Long-poll is `getUpdates` with
a timeout; webhook is an HTTP server the bot receives POSTs on.

- **Long-poll** is simpler (no port binding, no TLS, no
  reverse-proxy config), works behind NAT, is trivial to test
  locally, and is the shape every Telegram tutorial starts with.
- **Webhook** is the production deployment shape (lower latency,
  no wasted polls, works at scale) but requires a public
  endpoint and a TLS cert.

**Leaning:** long-poll only in Phase 8. The adapter's core is
the message-to-turn-loop plumbing, which is identical between
long-poll and webhook; a webhook is a transport swap, not a
redesign. Phase 9 or 10 can add webhook mode as a
`TelegramChannel::Mode` field without touching the turn-loop
interaction. Name this explicitly as a non-goal below.

### Q6. Does the Telegram adapter need anything out of D2 that the trait doesn't already give it?

**Status:** open at phase entry. Must resolve during Task 1.
**This is the other streak-ender candidate.**

The D2 `ChannelContext` trait today exposes:
`channel_name()`, `platform()`, `trust_tier()`, and — critically
— nothing about per-message state. The trait is stateless by
design. But a Telegram adapter needs to know:

- which chat the current message came from,
- which user (for future `session:<user_id>` scoping),
- whether the message is a `/cancel` command,
- message delivery status for the outgoing reply.

Three shapes for passing this to the turn loop:

1. **Out-of-band via an `Arc<Mutex<T>>` field on the channel
   impl.** The adapter stashes the current chat ID in a mutex
   when it invokes the turn loop, the turn loop reads it if it
   needs to. Hacky; stateless-by-design trait with a mutable
   side channel is a code smell.
2. **Extend `ChannelContext` with per-message accessors.**
   Methods like `current_chat_id()` / `current_user_id()`. D2
   amendment, ends the streak.
3. **Pass the context as a struct alongside the turn input.**
   A new `TurnContext { chat_id: Option<...>, user_id:
   Option<...> }` added to `run_session`'s signature, populated
   by the adapter. No trait change; the plumbing lives at the
   `run_session` call site.

**Leaning:** (3). The first time a second network channel shows
up, option (2) will be revisited — but option (3) keeps the
amendment deferred and, critically, keeps the contract
*true* rather than just *unchanged*. The Phase 6 Q5 rule
applies here literally: **honesty over streak preservation**.
If Task 1 reveals that (3) is structurally impossible — e.g.,
the turn loop needs to call back into the channel to check
cancellation and the callback needs the chat ID — then (2) is
the right outcome and the streak breaks at seven.

### Q7. Telegram Bot API library choice

**Status:** open at phase entry. Must resolve before Task 1.
**Nice-to-have; may defer to "whichever builds cleanest."**

Two credible crates:

1. **`teloxide`.** The community standard for Rust Telegram
   bots. Large dep set, opinionated about how you structure a
   bot (its `dptree` dispatcher is its whole value prop), good
   docs, mature.
2. **`frankenstein`.** Thin Bot API wrapper. Minimal deps,
   unopinionated, you write your own dispatch loop. Closer to
   what Phase 8 actually wants — the adapter is thin, the turn
   loop is the dispatcher.

**Leaning:** `frankenstein`. Phase 8's turn loop is the
dispatcher already (the LLM planner decides what tools to call,
not a rule-based message router), so `teloxide`'s dispatch
machinery is weight we'd fight against. A ~200-line hand-rolled
long-poll loop over `frankenstein` is probably smaller and
cleaner than wiring `teloxide`'s dispatcher around the turn
loop.

**Counterargument:** `teloxide` has battle-tested handling of
the Telegram API's edge cases (rate-limiting, retry,
`getUpdates` offset management, reconnect). Phase 8 will have
to re-implement whatever slice of that surface it ends up
needing. If three of those edge cases get hit during Task 1,
the leaning flips.

## Exit criteria (draft — revised as work lands)

- [ ] `crates/aivyx-telegram` exists with a `TelegramChannel`
      struct implementing `ChannelContext`, `platform() ==
      Telegram`, `trust_tier() == Untrusted`, and unit-test
      coverage of: single-message round trip, concurrent chats,
      mid-turn cancellation, mock HTTP transport failure paths.
- [ ] `aivyx --channel telegram` (or whatever Q4 resolves to) is
      a real binary surface that reads the bot token from the
      source Q1 picks, long-polls `getUpdates`, routes messages
      through `run_session` with a Telegram channel context,
      and sends replies via `sendMessage`. Smoke-tested against
      a real Telegram test bot.
- [ ] Per-chat memory isolation works: two chats under the same
      bot token cannot read each other's memory, asserted by
      an integration test whose whole job is to prove this
      invariant by signature rather than by trust.
- [ ] The persistent audit chain from Phase 7 continues to work
      unchanged — every turn over the Telegram channel lands in
      the same chain as every local turn, and
      `aivyx --verify-only` reports the combined count after a
      mixed local + Telegram session.
- [ ] Scope attenuation at the channel boundary is wired and
      has a negative test: a tool call that would succeed over
      `LocalChannel` fails over `TelegramChannel` because the
      tier attenuation narrowed the capability set.
- [ ] `cargo test --workspace` green. Net test-count delta ≥
      +20 (the heuristic from Phase 7's lessons: zero new tests
      = suspicious refactor).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
      clean.
- [ ] **Either** `DESIGN.md` is still unchanged (streak rolls to
      eight) **or** a single amendment file under
      `docs/amendments/` documents whichever of Q2 / Q3 / Q6
      required a contract change, with a pointer from the
      affected D2 / D4 text. Either outcome is acceptable —
      honesty over streak preservation, the Phase 6 Q5 rule.
- [ ] Q1 (token source), Q2 (per-chat identity), Q3 (scope
      attenuation location), Q4 (binary surface shape), Q5
      (long-poll vs webhook), Q6 (D2 per-message state), and
      Q7 (library choice) resolved and noted under "Decisions
      made during Phase 8 that aren't in DESIGN.md" in the
      freeze doc — regardless of which option won.
- [ ] At least one Phase 7 deferred item either landed or
      explicitly re-re-queued to Phase 9+ with a reason.
      Session-scoped memory qualifiers are the headline one
      — they're effectively required by Q2's option (2).
      `CapabilitySet::default()` ergonomics is **not**
      re-promised: it lands opportunistically if a Phase 8 task
      naturally touches the surface, or it rolls to Phase 9
      without comment.
- [ ] Phase 9 roadmap entry refined with whatever Phase 8
      uncovered about the trait-level seams a second adapter
      needs.

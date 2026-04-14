# Adapter pattern — how to add a new `ChannelContext`

This document is the **future-proof checklist** for adding a new
channel adapter to Aivyx. It was written at Phase 9 exit with exactly
two adapters in the tree (`LocalChannel` in `aivyx-channel` and
`TelegramChannel` in `aivyx-telegram`). Every claim below points at a
concrete line in one of those two adapters so a future contributor can
copy shapes rather than re-derive them.

Status: **tentative**. Two data points is not a pattern in the
software-architecture sense — it's a hypothesis waiting for a third
adapter to either confirm or break. Phase 9's explicit choice (Q1 =
Fork B) was to write the pattern down rather than stress-test it with
a third adapter *now*, on the bet that the next phase (or the first
phase that ships a third adapter) can confirm-or-refute with data. If
your third-adapter work breaks one of the rules below, the right move
is to update this document in the same commit, not to work around the
rule — the Phase 6 Q5 convention ("honesty over streak preservation")
applies.

## The trait surface

`ChannelContext` lives in `crates/aivyx-core/src/lib.rs:161-199`. It
has eight methods; the first seven are the D2 contract and the eighth
(`session_partition`) was added in Phase 8 Task 2 as a non-breaking
default:

| Method                 | Purpose                                          |
|------------------------|--------------------------------------------------|
| `channel_name()`       | Human-readable tag for audit / logs.             |
| `platform()`           | Enum the audit chain records per turn.           |
| `trust_tier()`         | Drives the capability ceiling (see below).       |
| `session_id()`         | Stable UUID per channel instance.                |
| `stream_event(event)`  | Tokens / status / tool markers during a turn.    |
| `finalize(outcome)`    | End-of-turn commit point.                        |
| `cancellation_token()` | Token the turn loop checks between LLM steps.    |
| `session_partition()`  | Optional per-instance memory partition.          |

The trait is `Send + Sync`. Implementations must hold any interior
mutation (buffers, token slots) behind a `Mutex` so the async
`&self`-taking methods can still mutate. **Never hold a `std::sync::
Mutex` across an `.await`.** Both existing adapters take this rule
seriously — the `TelegramChannel::finalize` path at
`crates/aivyx-telegram/src/telegram_channel.rs:243-277` drains the
buffer under the lock and explicitly drops the guard *before* the
network call, with a comment calling out that tokio doesn't understand
std mutexes.

## The tier-ceiling contract

A channel's `trust_tier()` is the entire capability story. When the
turn loop runs, it fetches the tier and intersects the agent's raw
capability set against `tier.default_ceiling()`:

```text
crates/aivyx-core/src/agent.rs:120
    let tier = channel.trust_tier();
    let effective = self.capabilities.intersect(tier.default_ceiling());
```

Whatever scopes the agent had on paper, the *effective* set for this
turn is whatever survives the intersection. A `Trusted`-tier channel
sees close to the full set; a `SemiTrusted` channel sees a narrower
slice; an `Untrusted` channel sees the smallest rung.

**Tier selection guidance.** D4 locks the four rungs (Kernel / Trusted
/ SemiTrusted / Untrusted); adapters do not get to invent a fifth. The
heuristic the two existing adapters use:

- **`Trusted`** — local user, direct process access. `LocalChannel` at
  `crates/aivyx-channel/src/local.rs:126-131` picks this because a
  CLI REPL runs as the user, in the user's shell, against the user's
  filesystem. Desktop GUI apps would sit here too.
- **`SemiTrusted`** — authenticated remote human. `TelegramChannel` at
  `crates/aivyx-telegram/src/telegram_channel.rs:212-218` picks this:
  the user is authenticated (Telegram's own login), but the channel
  crosses the network and runs under the bot token's identity rather
  than the user's machine identity. Matrix, Signal, Discord, Slack
  DMs all live here.
- **`Untrusted`** — anonymous or drive-by traffic. An unauthenticated
  HTTP POST endpoint, a public chatroom with no membership gating, a
  webhook from an external service. No adapter currently ships at
  this tier.
- **`Kernel`** is reserved for the turn loop's own bookkeeping; do
  not implement a channel at this rung.

If you find yourself wanting a fifth rung, stop and re-read the
heuristic — usually the answer is "your channel is `SemiTrusted` but
the *tool* you're worried about should attenuate its own scope," not
"the tier ladder is wrong." If the answer is genuinely "the tier
ladder is wrong," that's a D4 amendment, not a workaround — see
`docs/README.md` for the amendment process.

## The sibling `run_*_session` pattern

Aivyx has two session drivers. They are **deliberately not a shared
function**:

- `crates/aivyx-channel/src/session.rs:150` — `run_session` drives
  the line-buffered stdin REPL.
- `crates/aivyx-telegram/src/session.rs:346` — `run_telegram_session`
  (and its transport-generic inner at `:382`,
  `run_telegram_session_with_transport`) drives a long-poll cursor
  loop.

The middle of both functions is identical copy:

```text
    let registry = config.tools;
    let planner_config = LlmPlannerConfig::new(config.model)
        .with_system_prompt(config.system_prompt)
        .with_max_tokens(config.max_tokens);
    let agent = ConcreteAgent::new(
        AgentId::new(),
        config.capabilities,
        registry,
        audit,
        move || Box::new(LlmPlanner::new(...)),
    );
```

Phase 8 considered extracting this into a `run_any_session<C:
ChannelContext>(channel, ...)` helper and **rejected the extraction**.
The reason: the outer loops are fundamentally shaped differently.
`run_session` reads one line from stdin, rotates the channel's
cancellation token, drives one turn, loops. `run_telegram_session`
holds a long-poll cursor, batches inbound updates from N chats, has
its own shutdown token separate from the per-turn one, and (Phase 9
Task 1) interleaves a `/cancel` scan with turn execution via
`tokio::select!`. A shared helper would have to parameterize over
"how do you get the next message" and "what do you do between turns"
and "what's your shutdown story," and the result was larger than the
duplicated ~50 lines it would have replaced.

**The rule:** break the sibling pattern *only* if a concrete fourth
adapter forces it. Two data points rejected the extraction; three
data points either confirm "this was never going to be shared" (keep
the sibling pattern permanently) or force the extraction (at which
point the third adapter is load-bearing evidence for why). Do not
extract on the two-data-point evidence already in tree — the rejection
is documented in `docs/PHASE_8.md` Task 1 and this document will be
updated when the extraction case actually lands.

### What to copy vs what to write fresh

If you're adding adapter #3, the copy-vs-fresh boundary is:

- **Copy verbatim** (~50 lines): the planner factory closure, the
  `ConcreteAgent::new` block, the `LlmPlannerConfig` construction.
  These are identical in both existing adapters and should stay
  identical in yours.
- **Write fresh**: the outer loop (how you get the next user
  message), the shutdown story (distinct from per-turn cancellation,
  if your adapter has one), and the finalize-side rendering (how
  `stream_event` buffers or flushes).

## The private `XxxTransport` trait seam

`TelegramChannel` is generic over a private trait:

```text
crates/aivyx-telegram/src/transport.rs:72
    pub(crate) trait TelegramTransport: Send + Sync {
        async fn get_updates(&self, offset: i64, timeout_secs: u32)
            -> Result<Vec<IncomingMessage>, TransportError>;
        async fn send_message(&self, msg: OutgoingMessage)
            -> Result<(), TransportError>;
    }
```

Exactly two methods. Two concrete impls: `ReqwestTransport` at
`crates/aivyx-telegram/src/transport.rs:111` wraps
`frankenstein::client_reqwest::Bot` for production; `ScriptedTransport`
in the crate's `tests` module is a deterministic double that captures
outgoing messages and replays scripted inbound batches.

**Why a private 2-method trait instead of generic'ing over the SDK's
own trait.** `frankenstein` exposes a 90-method `AsyncTelegramApi`
trait that the reqwest client implements. `TelegramChannel` could
have been generic over that and skipped the wrapper. It deliberately
doesn't:

1. **Surface narrowing.** The channel consumes exactly two Bot API
   methods. Pinning the surface to those two means a `frankenstein`
   upgrade that adds a method cannot accidentally break the test
   double, and the test double is ~40 lines instead of ~400.
2. **Error-shape collapse.** The SDK returns a rich error enum. The
   seam collapses everything to a single `TransportError::Platform
   (String)` so the channel's error handling is uniform.
3. **Dependency hygiene.** The private trait means the SDK type
   `frankenstein::Bot` never appears in a `TelegramChannel` method
   signature, so `aivyx-telegram`'s public surface is free of
   frankenstein references. Reverse-fan-in is minimized.

**For adapter #3:** create a private `pub(crate) trait XxxTransport`
in your crate with exactly the methods you actually call. Production
impl wraps whatever SDK you're using. Tests impl is a scripted
double living in the tests module or a `src/tests.rs` file. If your
adapter is SDK-free (e.g., raw `reqwest` against a REST endpoint),
the transport trait is still worth it — your scripted double captures
the request/response pairs without standing up a mock HTTP server.

## `session_partition` and the multi-tenant story

A single process running `aivyx --channel telegram` can serve N
Telegram chats. Each chat needs its own memory namespace or chats
will see each other's `memory.read`/`memory.write` output. The
mechanism is `ChannelContext::session_partition()`, which returns
an opaque `Option<String>`:

- **`LocalChannel` inherits the default `None`** at
  `crates/aivyx-core/src/lib.rs:196-198`. One local process, one user,
  one partition. This is Phase 9 Q6's resolution (Option A — keep
  `None`): Phase 6's cross-restart recall story depends on local
  memory being shared across invocations, and the per-terminal-
  partition shape (Option B) would regress that. If a future local
  identity boundary appears, upgrade to Option C (stable per-machine
  identifier) at that time.
- **`TelegramChannel` returns `Some(chat_id.to_string())`** at
  `crates/aivyx-telegram/src/telegram_channel.rs:224-232`. Each chat
  is its own partition. The stringified `chat_id` is Telegram's
  authoritative, stable identity for a conversation.

The turn loop injects the partition into tool input JSON *before*
`required_scope` runs:

```text
crates/aivyx-core/src/agent.rs:314-330
    if let Some(partition) = channel.session_partition()
        && let Some(obj) = input.as_object_mut()
    {
        obj.insert("session".to_string(), serde_json::Value::String(partition));
    }
```

This injection is load-bearing for three reasons:

1. The tool's `required_scope` function sees the `session` field and
   derives a dual-qualifier scope like `memory.read:topic:notes:
   session:12345`. The audit chain records that full scope, so
   per-chat evidence lands in the chain alongside per-turn evidence.
2. The tool's `execute` function sees the same `session` field and
   routes to a physical topic key via `namespaced_topic()` (see next
   section). Same function sees same shape — the gate and the
   executor can't disagree.
3. The LLM never sees this field. It is not in any advertised
   `input_schema` and is added after the planner emits the tool
   call. A misbehaving LLM cannot forge a `"session"` field that
   controls which partition it reads, because the turn loop
   overwrites whatever the LLM emitted with the channel's
   authoritative partition.

**For adapter #3:** decide at channel construction time what your
partition identity is. If your adapter has one identity per instance
(like `LocalChannel`), inherit the default and return `None`. If it
has many (like `TelegramChannel`, one per chat), return
`Some(stable_id_string)`. The string is opaque to the turn loop — the
only requirement is that it's stable for the life of the channel
instance and unique across instances that should not see each other's
state.

## `namespaced_topic` stays in `aivyx-memory`

`namespaced_topic` at `crates/aivyx-memory/src/tools.rs:192` is the
helper that turns a logical topic + optional session into a physical
byte string (`\x01s\x01<session>\x01<logical>`). It uses ASCII `0x01`
as a prefix marker that's unusable in well-formed topics, so
namespaced and non-namespaced keys cannot collide.

Phase 9 Q5 asked whether this helper should move to `aivyx-capability`
so non-memory tools can partition their state too. **Resolution: it
stays where it is.** The reasoning:

- Memory is currently the only tool family whose state is
  *partitioned by session*. `fs.read` / `fs.write` use the per-process
  `fs_root` and are not per-chat scoped. Shell-exec tools (when they
  land) should not be scoped to a chat at all. LLM-provider tools
  don't carry state.
- Promoting `namespaced_topic` to `aivyx-capability` would be speculative
  generalization — no current consumer needs it there, and
  `aivyx-capability` already owns `Scope` + `TrustTier`, which are the
  cross-tool primitives. Partition namespacing is substrate-specific
  (redb flat-topic-map shape), not a capability primitive.

If a future tool family contradicts this — for example, a
per-chat-scoped `shell.exec` state-tracker — promote the helper at
that time. Until then, `aivyx_memory::tools::namespaced_topic` is
pub-visible inside the crate only, and the pattern is "if your tool
needs per-session state, write your own `namespaced_key` helper in
your own crate using the same `\x01`-prefixed layout."

## Per-task clippy policy

Phase 8 Task 8 established the convention that every task runs `cargo
clippy --workspace --all-targets -- -D warnings` before shipping, not
just at phase exit. Phase 9 Task 3 backed this with a pre-commit hook
at `scripts/pre-commit.sh`, installable via `scripts/install-hooks.sh`.

**For adapter #3:** the hook catches you for free if you've run the
installer. If you haven't, run clippy by hand before each commit. A
dirty clippy run that survives into `main` is the shape of bug
Phase 8 Task 4 shipped, Task 8 caught, and Task 3's hook now blocks
— don't reopen the gap.

## The zero-core-touch target

Both Phase 8 (Telegram adapter) and Phase 9 (config, `/cancel`,
multi-chat) have held the invariant that `crates/aivyx-core/` stays
unchanged. Phase 8 Task 2 made one exception — a non-breaking
additive `session_partition()` default method with a matching
injection site in `agent.rs` — and it was called out in the commit
message and PHASE_8.md Task 2 "Streak impact" section as a deliberate
contract extension, not a contract amendment.

**For adapter #3:** aim for zero touches to `aivyx-core`. The turn
loop, `ChannelContext` trait, and audit chain are load-bearing
primitives that every other adapter depends on. If your adapter
thinks it needs a core change, stop and check whether the change can
land as a new method with a default (like `session_partition()` did)
or as an opt-in trait extension. If it genuinely needs a breaking
change, that's a D2 amendment — file one. Do not rename or reshape
existing methods to fit your adapter's preferences.

## The checklist

When you sit down to add adapter #3, the concrete steps:

1. **Create the crate.** `crates/aivyx-<platform>/` with
   `Cargo.toml`, `src/lib.rs`, `src/<platform>_channel.rs`,
   `src/transport.rs`, `src/session.rs`, `src/tests.rs`. Four module
   files + tests is the shape `aivyx-telegram` landed on.
2. **Write the private transport trait.** Two-to-four methods
   covering only what your channel calls. Production impl wraps the
   SDK. Test impl is a scripted double with a capture buffer.
3. **Write the `ChannelContext` impl.** Pick your trust tier from
   the table above. Decide whether `session_partition()` returns
   `None` or `Some(stable_id)`. Hold any buffers / token slots behind
   `std::sync::Mutex` and never hold the lock across an `.await`.
4. **Write the session driver.** Copy the planner/agent construction
   from `run_session` or `run_telegram_session_with_transport`
   verbatim. Write your own outer loop. Rotate the channel's
   cancellation token between turns via a `reset_cancellation()`
   method on your channel.
5. **Write the scripted e2e test.** Drive the transport double
   through at least a round-trip and a cancellation case. Use the
   `aivyx-telegram` `src/tests.rs` `two_chats_persistent_e2e` test
   as a reference for multi-partition coverage. Hit the real
   persistent audit chain in the test — mock audits don't catch the
   bugs persistent audits do.
6. **Wire the binary.** Add a `ChannelKind::<Yours>` variant to the
   `aivyx` binary's channel dispatch. `aivyx-config` already owns
   env-var / TOML / encrypted-store config loading, so your adapter
   plugs into the existing shape rather than inventing its own
   env-var vocabulary.
7. **Defer the real-protocol smoke test** to the Channel Activation
   Milestone (see `docs/ROADMAP.md`). Do not try to credentialize a
   real adapter at ship time; the scripted transport covers enough
   to ship, and the real-protocol pass runs once per milestone
   against the full adapter matrix.
8. **Run clippy before every commit.** Install the pre-commit hook
   if you haven't.
9. **Update this document** if any step above felt wrong, and say
   so in your adapter's ship commit. Two-data-point patterns become
   three-data-point patterns by someone explicitly writing down what
   the third data point taught us.

## Known unresolved questions

- **When does the sibling pattern break?** Unresolved at Phase 9
  exit. Two data points kept it; a third will either confirm or
  break. See PHASE_9.md Q1's Fork A vs Fork B discussion for the
  framing.
- **Does the partition type need to be richer than `Option<String>`?**
  PHASE_9.md Q7 left this for the first adapter with structured
  identity (e.g., Matrix: room_id + homeserver). Phase 9 didn't
  force the issue. If adapter #3 is Matrix, start here.
- **Does the per-chat shutdown token story scale past one
  multiplexer?** Phase 9 Task 2's multi-chat pumping uses one outer
  shutdown token + per-turn rotation per chat. A future adapter
  with a different connection model (persistent websocket, gRPC
  stream) may need a different story — revisit at that point.

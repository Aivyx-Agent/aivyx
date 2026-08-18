# Wire the remaining `aivyx-checkpoint` construction sites

_2026-08-19._ Follow-on to `aivyx-checkpoint`'s adoption into `aivyx`
(`aivyx/docs/superpowers/specs/2026-08-18-aivyx-checkpoint-adoption-design.md`),
which wired 4 of 9 real `ConcreteAgent` construction sites
(`daemon_agent`, `child_agent`, and — added mid-flight after that
project's own final review — the default no-daemon REPL and voice paths
via `aivyx-channel::build_agent_stack`). This project closes the
remaining gap logged in `aivyx-ecosystem/ROADMAP.md`'s "New backlog"
note under `aivyx-checkpoint`'s entry.

## The backlog note's own scope estimate was wrong — corrected here

The prior project's backlog note named "7 other real construction
sites": the standalone Discord/Slack/Telegram bot-mode session builders,
`aivyx-channel::build_agent_stack` "when called by anyone other than
`aivyx.rs`," and `aivyx-team`'s factory/CLI module. Investigating before
designing (not trusting the prior note's own summary — the same
discipline that caught three real errors in the *original* combined
design during the adoption project) found two corrections:

1. **`aivyx-discord`/`aivyx-slack`/`aivyx-telegram` are not standalone
   binaries.** None of the three crates has a `[[bin]]` target. Their
   session functions (`run_discord_session`, `run_slack_session`,
   `run_telegram_multi_session`) are called directly from `aivyx.rs`'s
   own `run()` — the exact same function that already builds
   `checkpointer` near `canonical_root` and threads it into
   `daemon_agent`/`child_agent`/REPL/voice. This is the same
   "thread a parameter through" shape as the adoption project's own
   Task 4, not a separate config-resolution problem the "standalone"
   framing implied.
2. **`build_agent_stack`'s "other caller" doesn't exist.** A
   workspace-wide `grep -rn "build_agent_stack("` (not scoped to any one
   crate's `src/` — the adoption project's own earlier grep mistake was
   exactly this kind of narrow scoping, missing `tests/` callers) finds
   exactly two real callers: `run_session` and the voice arm's call in
   `aivyx.rs`. Both are already fixed. This backlog line was already
   fully resolved by the time it was written down — nothing to do here.

The real remaining count is **5 sites**, not 7:

- `aivyx-discord::run_discord_session_with_mailbox`'s `ConcreteAgent::new`
  (three hops deep: `run_discord_session` → `run_discord_session_with_transport`
  → `run_discord_session_with_mailbox`, the last `pub(crate)`, generic
  over transport, where the real construction happens).
- `aivyx-slack`, the identical three-hop shape
  (`run_slack_session` → `run_slack_session_with_transport` →
  `run_slack_session_with_mailbox`).
- `aivyx-telegram`, the identical three-hop shape for the **multi**-session
  variant (`run_telegram_multi_session` → `run_telegram_multi_session_with_transport`
  → presumably an internal construction site reached the same way).
  `run_telegram_session` (singular) has **zero** real callers anywhere in
  the workspace — confirmed dead in production, not touched by this
  project.
- `aivyx-team::SpecialistFactory::build` — the real production path for
  every team specialist agent, reached via `SpecialistPool::spawn`
  (`crates/aivyx-team/src/pool.rs:258`).
- `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`'s `run_mission` — the
  CLI's own `team run` command's lead agent, called from `aivyx.rs`'s
  `run()` at (currently) line 7693, textually **after** `checkpointer`'s
  own construction (~line 6001) in the same function — pending
  confirmation during implementation that no early return or conditional
  branch between those two lines removes it from scope for this call
  path specifically (the same category of risk Tasks 3/4 of the prior
  project each flagged once and resolved via a documented fallback, not
  a blocker to designing around now).

## Design

**Same instance, threaded as a parameter, mirroring the adoption
project's Task 4 exactly.** No new `GitCheckpointer` construction
anywhere in this project — every site receives `checkpointer.clone()`
(or the bare binding, for the last user in a given scope) from the one
instance `aivyx.rs` already builds near `canonical_root`.

### 1. `aivyx-discord` / `aivyx-slack` / `aivyx-telegram`

Each crate's three-function chain gains a `checkpointer:
Option<Arc<aivyx_core::GitCheckpointer>>` parameter, threaded from the
public entry point down to the `pub(crate)` `*_with_mailbox` function
where `ConcreteAgent::new(...)` is actually called — add
`.with_checkpointer(checkpointer)` to that call, matching the exact
builder-chain position used at every other site in this branch's history
(`agent.rs`'s `with_budget_gate`/`with_rate_gate`/`with_checkpointer`
convention).

`aivyx.rs`'s three call sites (`aivyx_discord::run_discord_session(...)`
around line 9656, `aivyx_slack::run_slack_session(...)` around line 9786,
`aivyx_telegram::run_telegram_multi_session(...)`) each gain
`checkpointer.clone()` as a new argument, in the same relative position
Task 4 used for `run_session`/`AgentStackSpec` (immediately after
`audit`, the pattern proven to compile cleanly and match every other
call site's convention in this codebase).

### 2. `aivyx-team::SpecialistFactory`

`SpecialistFactory` gains a `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>`
field, defaulted to `None` in `SpecialistFactory::new` (matching
`dialogue: None` in the same constructor — an optional feature set after
construction, not a constructor argument), with a new builder method
(e.g. `with_checkpointer`, mirroring `ConcreteAgent`'s own builder
convention) to set it. `SpecialistFactory::build`'s `ConcreteAgent::new(...)`
call gains `.with_checkpointer(self.checkpointer.clone())`.

Wherever `aivyx.rs` constructs the real `SpecialistFactory` used by
`SpecialistPool` (the "Chapter L (L.5)" team-mission block, near where
`team_missions`/`TeamRunDeps` are built) gains a `.with_checkpointer(...)`
call using the same `checkpointer` binding.

### 3. `aivyx_modules/team.rs::run_mission`

Gains a `checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>`
parameter, used in its own `ConcreteAgent::new(...).with_checkpointer(...)`
call. `aivyx.rs`'s call site (line ~7693) passes `checkpointer.clone()`.
If implementation finds `checkpointer` genuinely out of lexical scope at
that call site (the one real open question this design carries forward
rather than resolving by assumption), the fallback is the same one Task 3
used for `child_agent`'s closure-capture issue: clone `checkpointer` into
an earlier, appropriately-named binding before whatever branch/closure
narrows scope, not restructure `run()`'s own control flow.

## Testing

One test per channel crate (`aivyx-discord`, `aivyx-slack`,
`aivyx-telegram`) proving a real dispatched mutating call through that
crate's own real construction chain (not a hand-built `ConcreteAgent`)
produces a checkpoint ref — mirroring each crate's own existing test
fixtures/mocked-transport patterns already used for their other
integration tests, not inventing a new fixture style. One test in
`aivyx-team` proving a specialist whose attenuated capabilities include
`fs.write` gets checkpointed when it writes, using `SpecialistFactory`'s
own existing test helpers (already present in `factory.rs`'s test
module, e.g. the `factory(base: Vec<Arc<dyn Tool>>)` helper used by its
existing `.build(...)` tests).

## Explicitly out of scope

- No new operator-facing config — this project adds zero TOML surface,
  matching the original adoption's own constraint.
- `git.rs`'s three tools and `workspace.*` tools remain unprotected —
  unchanged scope decision from the original adoption design.
- Any restructuring of the 3-hop `run_X_session` → `_with_transport` →
  `_with_mailbox` call chains beyond adding the one new parameter each —
  this is a pure additive threading pass, not a refactor.

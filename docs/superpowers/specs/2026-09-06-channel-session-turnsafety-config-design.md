# Thread `[agent]` Config Into the Standalone Channel-Session Fallback Paths

**Status: approved, ready for implementation planning.**

## Context

Phase 200 propagated `injection_scan_enabled`/`injection_scan_exempt`
through every real `TurnSafety::interactive`/`autonomous` call site in the
system, and its own final review found 4 call sites that construct agents
via `TurnSafety::default()` directly, never going through either
constructor at all: `crates/aivyx-discord/src/session.rs:214`,
`crates/aivyx-slack/src/session.rs:162`, and
`crates/aivyx-telegram/src/session.rs:436`/`:807`. Phase 200 fixed the
resulting security regression (a hand-written `Default` impl keeping
`injection_scan_enabled: true`) but correctly left these 4 sites
unconfigurable — they were out of that phase's stated scope. This phase
closes that gap: give these paths the *same* `[agent]` config every other
agent in the system already receives, not just a safe default.

**Grounding confirmed the real shape of the gap is smaller than it might
sound:**

- Each of the 3 channel crates has **two dispatch modes**: a **daemon-mode**
  path (a thin IPC client relaying to an already-running daemon process —
  `telegram_daemon_frontend.rs`/`discord_daemon_frontend.rs`/
  `slack_daemon_frontend.rs` in `aivyx-channel`, none of which construct a
  `ConcreteAgent` or reference `TurnSafety` at all) and an **in-process
  fallback** (used when `--no-daemon` is passed or no daemon socket is
  reachable). The daemon-mode path was never at risk — the daemon process
  itself constructs `daemon_agent`, already correctly wired since Phase
  199/200. **Only the in-process fallback ever calls `TurnSafety::default()`
  directly**, and that's the entire surface this phase touches.
- `TelegramSessionConfig` (`aivyx-telegram`), `DiscordSessionConfig`
  (`aivyx-discord`), and `SlackSessionConfig` (`aivyx-slack`) are
  structurally identical: same 8 fields (`model`, `system_prompt`,
  `max_tokens`, `capabilities`, `tools`, `storage`, `tool_allowlist`,
  `memory_topic_prefix`), each `#[derive(Clone)]`, each with **exactly one**
  real production construction site, all three inside the same giant
  `run_async` function in `crates/aivyx-cli/src/bin/aivyx.rs`
  (`TelegramSessionConfig` at line 9931, `DiscordSessionConfig` at line
  10043, `SlackSessionConfig` at line 10180) — the same scope
  `injection_scan_enabled`/`injection_scan_exempt`/`turn_timeout_secs`/
  `cycle_detection` are already unwrapped into as local bindings (confirmed:
  `injection_scan_enabled`/`injection_scan_exempt` at lines 6390–6391,
  `turn_timeout_secs`/`cycle_detection` destructured at lines 5786/5789,
  and all four already flow into the `daemon_agent`/`child_agent`/
  `AgentStackSpec` sites at lines 8620–8621, 9125–9126, 9830–9831, and
  10320–10321). **No new config plumbing is needed anywhere** — this is a
  threading fix, exactly like Phase 199/200's own work, not new wiring.
- Telegram has two `TurnSafety::default()` sites (`:436` in
  `run_telegram_session_with_transport`, the single-chat path; `:807` in
  `run_telegram_session_with_mailbox`, the multi-chat path) but **both
  consume the same `TelegramSessionConfig` type** — fixing that one struct
  fixes both call sites. As a side effect this also fixes
  `run_telegram_session` (the public single-chat entry point), which
  grepping confirmed has **zero real callers today** — Telegram's actual
  CLI dispatch (`aivyx.rs:9941`) only ever calls `run_telegram_multi_session`.
  Fixing it costs nothing extra and leaves no asymmetry in the crate's own
  public API.
- `checkpointer: Option<Arc<GitCheckpointer>>` is threaded as a **separate
  function parameter** to `run_telegram_multi_session`/`run_discord_session`/
  `run_slack_session` (not a `SessionConfig` field) — that precedent doesn't
  apply here. The values this phase threads are operator **config**, not a
  shared resource handle, so the right precedent is `tool_allowlist`/
  `memory_topic_prefix`, which already live as plain fields on these same
  3 structs. Confirmed with the user before writing this spec.
- These are genuinely interactive sessions — a human operator is on the
  other end of a live Telegram/Discord/Slack conversation, one turn per
  inbound message, the exact same shape as `daemon_agent`/`child_agent`
  (both already `TurnSafety::interactive(...)`). `TurnSafety::autonomous()`
  (team missions, no human in the loop) does not apply here — there is no
  posture ambiguity to resolve.
- `turn_timeout_secs: Option<u64>` and `cycle_detection: Option<bool>` are
  both `Copy` (confirmed against `aivyx-config/src/lib.rs:1112`/`:1116`) —
  no ownership concern threading them into 3 separate match arms.
  `injection_scan_exempt: BTreeSet<String>` is **not** `Copy`; grounding
  confirmed each of the 3 arms (`ChannelKind::Telegram`/`Discord`/`Slack`
  in `aivyx.rs`'s dispatch `match`, lines 9844/9961/10082) uses it exactly
  once, with no reuse later in the same arm or after the match — so it
  should be **moved**, not cloned, into each config literal, matching how
  `tool_allowlist`/`memory_topic_prefix` (also non-`Copy`) are already
  moved into these same 3 struct literals today.

## Approach

Add 4 fields — `turn_timeout_secs: Option<u64>`, `cycle_detection:
Option<bool>`, `injection_scan_enabled: bool`, `injection_scan_exempt:
BTreeSet<String>` — to `TelegramSessionConfig`, `DiscordSessionConfig`, and
`SlackSessionConfig`, placed and doc-commented in the same style as the
existing `tool_allowlist`/`memory_topic_prefix` fields on each struct.
Change each of the 4 `TurnSafety::default().apply(agent)` call sites to
`TurnSafety::interactive(config.turn_timeout_secs, config.cycle_detection,
config.injection_scan_enabled, config.injection_scan_exempt.clone()).apply(agent)`
— matching the exact call shape `daemon_agent`/`child_agent` already use.
Populate the 4 new fields at the 3 real `aivyx.rs` construction sites from
the already-in-scope locals.

### 1. `TelegramSessionConfig` (`crates/aivyx-telegram/src/session.rs`)

```rust
#[derive(Clone)]
pub struct TelegramSessionConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub capabilities: CapabilitySet,
    pub tools: Arc<ToolRegistry>,
    pub storage: Arc<dyn Storage>,
    pub tool_allowlist: Option<std::collections::BTreeSet<String>>,
    pub memory_topic_prefix: Option<String>,
    /// Chapter Bridle (BR.4) — `[agent] turn_timeout_secs` override.
    /// Threaded into `TurnSafety::interactive(...)` at each construction
    /// site below, matching `daemon_agent`/`child_agent`. `None` preserves
    /// the turn loop's built-in default deadline.
    pub turn_timeout_secs: Option<u64>,
    /// `[agent] cycle_detection` — arm the small-cycle breaker. Threaded
    /// into `TurnSafety::interactive(...)` alongside `turn_timeout_secs`.
    pub cycle_detection: Option<bool>,
    /// `[agent] injection_scan_enabled` — Chapter Picket's active-scan
    /// on/off. See `aivyx_core::TurnSafety` for the full contract.
    pub injection_scan_enabled: bool,
    /// `[agent] injection_scan_exempt` — per-tool-name exemption list for
    /// the active scan. See `aivyx_core::TurnSafety` for the full contract.
    pub injection_scan_exempt: std::collections::BTreeSet<String>,
}
```

Both `run_telegram_session_with_transport` (line ~417) and
`run_telegram_session_with_mailbox` (line ~787) change their
`TurnSafety::default().apply(agent)` call (lines 436, 807) to:

```rust
let agent = aivyx_core::TurnSafety::interactive(
    config.turn_timeout_secs,
    config.cycle_detection,
    config.injection_scan_enabled,
    config.injection_scan_exempt.clone(),
)
.apply(agent);
```

Both functions already bind `config: TelegramSessionConfig` (consumed by
value earlier in the same function for `config.tools`/`config.storage`/
etc.) — `config.injection_scan_exempt.clone()` is required since `config`
is otherwise fully destructured by this point; confirm the exact field
access pattern against the surrounding code at implementation time (it may
already be a partial move, in which case capture `injection_scan_exempt`
into a local before the destructuring begins, the same way `tool_allowlist`
is separately captured via `config.tool_allowlist.clone()` earlier in
`run_telegram_session_with_transport`).

The stale comments immediately above both call sites (*"This standalone
path carries no `[agent]` config to inherit, so it stays at the built-in
defaults... a future config thread switches this to
`TurnSafety::interactive(...)` in one place"*) are replaced with a comment
matching `daemon_agent`'s own: *"Route through the shared per-turn-safety
choke point with the operator's configured values."*

### 2. `DiscordSessionConfig` (`crates/aivyx-discord/src/session.rs`)

Same 4 fields, same doc comments, added to `DiscordSessionConfig`. The one
`TurnSafety::default().apply(agent)` call (line 214, inside
`run_discord_session_with_mailbox`) becomes the same
`TurnSafety::interactive(config.turn_timeout_secs, config.cycle_detection,
config.injection_scan_enabled, config.injection_scan_exempt.clone())`
call, with the same stale-comment replacement.

### 3. `SlackSessionConfig` (`crates/aivyx-slack/src/session.rs`)

Same 4 fields, same doc comments, added to `SlackSessionConfig`. The one
`TurnSafety::default().apply(agent)` call (line 162, inside
`run_slack_session_with_mailbox`) gets the same treatment.

### 4. The 3 real construction sites (`crates/aivyx-cli/src/bin/aivyx.rs`)

Each of the 3 `ChannelKind::{Telegram,Discord,Slack}` match arms' config
literal (lines 9931, 10043, 10180) gains the 4 new fields, populated from
the locals already in scope:

```rust
let telegram_config = TelegramSessionConfig {
    model,
    system_prompt,
    max_tokens: DEFAULT_MAX_TOKENS,
    capabilities,
    tools,
    storage,
    tool_allowlist,
    memory_topic_prefix,
    turn_timeout_secs,
    cycle_detection,
    injection_scan_enabled,
    injection_scan_exempt,
};
```

(Same shape for `discord_config`/`slack_config`.) `turn_timeout_secs`/
`cycle_detection` are `Copy`, so no ownership concern reusing them across
the 3 mutually-exclusive match arms. `injection_scan_enabled` is `Copy`.
`injection_scan_exempt` is moved (not cloned) into whichever arm's config
literal — confirmed at grounding time that neither the arm itself nor any
code after the match reuses it.

## Testing

- **`aivyx-telegram`**: update the existing `TelegramSessionConfig { ... }`
  test fixtures (`crates/aivyx-telegram/src/tests.rs` — 8 real
  construction sites found by `grep`, lines 983, 1334, 1797, 1807, 2277,
  2532, 2824, 3255) to include the 4 new fields, using
  `turn_timeout_secs: None, cycle_detection: None, injection_scan_enabled:
  true, injection_scan_exempt: BTreeSet::new()` (the default-preserving
  values, matching how `TurnSafety::default()`'s own hand-written impl from
  Phase 200 preserves prior behavior) unless a specific test's own point is
  to exercise one of these knobs. Add one new test:
  `run_telegram_session_with_mailbox` (or `_with_transport`) constructed
  with `injection_scan_enabled: false` produces a turn that does **not**
  escalate on an injection-marker-bearing tool output — mirroring Phase
  199's own `injection_scan_disabled_globally_skips_the_scan_but_still_fences`
  test pattern, adapted to go through `TelegramSessionConfig` instead of a
  direct `ConcreteAgent` builder call.
- **`aivyx-discord`**: update the one `DiscordSessionConfig { ... }` test
  fixture (`crates/aivyx-discord/src/tests.rs:167`) the same way. One new
  test, same shape as Telegram's.
- **`aivyx-slack`**: update the one `SlackSessionConfig { ... }` test
  fixture (`crates/aivyx-slack/src/tests.rs:144`) the same way. One new
  test, same shape as Telegram's.
- **`aivyx-cli`**: no new unit tests planned — the 3 construction sites are
  simple field-literal additions from already-in-scope locals, covered by
  the crate-level tests above plus the existing `cargo build`/`cargo test`
  verification every task already runs.

## Self-review

- **Placeholder scan:** none — every struct field, doc comment, and
  call-site change is given concretely, grounded against the real current
  file content of all 3 session crates and all 3 `aivyx.rs` construction
  sites.
- **Internal consistency:** all 4 real `TurnSafety::default()` call sites
  (Telegram×2, Discord×1, Slack×1) end up calling the identical
  `TurnSafety::interactive(...)` signature with the same 4 arguments in
  the same order, matching `daemon_agent`/`child_agent`'s existing calls
  exactly — no new constructor shape introduced.
- **Scope check:** one phase, 3 small structs across 3 crates (each
  gaining 4 fields) + 4 call-site edits + 3 `aivyx.rs` literal edits +
  8+1+1 test-fixture updates. Smaller than Phase 200 (no multi-crate
  builder-propagation chain like `SpecialistFactory`/`TeamAssembly` —
  every touched type already has the exact field it needs, this only adds
  4 more of the same shape).
- **Ambiguity check:** the field-placement question (config-struct fields
  vs. separate function parameters, mirroring `tool_allowlist` vs.
  `checkpointer`) was confirmed with the user, not assumed. The
  `.interactive()` vs `.autonomous()` posture has no real ambiguity — these
  are human-driven conversational sessions, structurally identical to the
  2 existing `.interactive()` call sites.

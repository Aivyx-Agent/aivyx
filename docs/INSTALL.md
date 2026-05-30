# Installing Aivyx

This doc covers the full install matrix. For the abbreviated
"Five-minute setup" path, see the [root README](../README.md).

Aivyx ships a single binary, `aivyx`, plus five optional channel
adapters baked into it (CLI, Telegram, Discord, Slack, Web UI).
There are no hosted dependencies — your binary talks directly to
your LLM provider (Anthropic / OpenAI-compatible / Ollama) and
stores everything locally in an encrypted redb file.

## Current install state

Phase 61 wired the release pipeline (cargo-dist, GitHub Actions
CI, four-target matrix, install-script generation) but did
**not** publish a release. Until public hosting is configured
and Task 7 lands, the only supported install path is
[build from source](#build-from-source-currently-the-only-path).

The shell-installer section below documents the path that will
become primary once `v0.1.0` is published.

## Supported targets

When the release pipeline fires, it will cover four targets. All
Linux builds are musl-static, so a single Linux binary works on
every distro without glibc version drift.

| Target | Binary | Notes |
|---|---|---|
| Linux x86_64 (musl) | `aivyx` | Debian 8+ / Ubuntu 16+ / Arch / Alpine / RHEL 7+ |
| Linux aarch64 (musl) | `aivyx` | ARM64 servers, Raspberry Pi 4/5 (64-bit OS), Asahi Linux |
| macOS x86_64 | `aivyx` | Intel Macs, macOS 10.13+ |
| macOS aarch64 | `aivyx` | Apple Silicon (M1 / M2 / M3 / M4), macOS 11+ |

Native Windows is **not yet supported**. The daemon's IPC layer
uses Unix domain sockets (mode 0600, OS-user identity); a Windows
port needs a NamedPipe replacement. Until that lands, run Aivyx
inside [WSL2](https://learn.microsoft.com/en-us/windows/wsl/) — it
behaves as a regular Linux x86_64 install.

## Build from source (currently the only path)

This is the supported install path today. Cargo build from a
clone of the repository:

**Prerequisites:**
- Rust toolchain 1.85+ (`rustup` recommended)
- A C linker (`gcc` / `clang` / Xcode CLT)
- For Linux musl builds: the `musl-tools` package (Debian) or
  equivalent

```sh
git clone https://github.com/AivyxDev/aivyx
cd aivyx
cargo build --release --bin aivyx
# binary lands at target/release/aivyx
```

Install into `~/.cargo/bin/` (if `cargo install` is preferred):

```sh
cargo install --path crates/aivyx-channel --bin aivyx
```

The pre-commit hook (`./scripts/install-hooks.sh`) is optional
for end users; it enforces `cargo clippy --workspace --all-targets
-- -D warnings` on every commit and is recommended for
contributors.

## Running Aivyx locally for development (Phase 99)

For a development loop — building from a clone and exercising the
real agent on your own machine — two scripts under `scripts/`
wrap the binary against a **fully local Ollama backend** (no API
key, no network egress, no per-run cost).

**Prerequisites:**
- [Ollama](https://ollama.ai) installed and running (`ollama serve`)
- A model pulled, e.g. `ollama pull llama3.1`

**Interactive session** — `scripts/dev-run.sh` builds `aivyx` and
drops you into a chat REPL:

```sh
./scripts/dev-run.sh                       # default model: llama3.1
./scripts/dev-run.sh --model llama3.2      # pick another pulled model
./scripts/dev-run.sh --reset               # wipe local state first
./scripts/dev-run.sh -- --role coder       # args after -- go to the binary
```

**Scripted verification pass** — `scripts/dev-verify.sh` (also
reachable as `dev-run.sh --verify`) runs a non-interactive battery
over the store, audit chain, daemon lifecycle, and the memory/fs
tool paths, printing a `PASS`/`WARN`/`FAIL` summary:

```sh
./scripts/dev-verify.sh --model llama3.1
```

Substrate checks (store, audit chain, daemon) are deterministic
and a failure exits non-zero. Tool-path probes depend on the
local model actually choosing to call a tool, so a miss there is
reported as `WARN`, not `FAIL`.

All state from both scripts lands under a gitignored `.dev-run/`
directory — a sandbox FS root, an encrypted dev store, and a
throwaway dev passphrase. It is disposable scratch state, never
real data; delete it freely or pass `--reset` for a clean start.

This local-run path is deliberately Ollama-only and leaves no
CI or remote-build footprint: Phase 99 keeps builds local while
repo infrastructure is still being decided.

## Shell installer (when published)

This section documents the path that becomes primary once
`v0.1.0` is published. **It does not work yet** — the URL
returns 404. The pipeline is in place to fire on the first tag
push to a public GitHub remote.

The cargo-dist-generated installer will detect your arch,
download the right tarball, verify its checksum, and drop
`aivyx` into `$CARGO_HOME/bin/` (typically `~/.cargo/bin/`).

```sh
# Will work post-publication:
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/AivyxDev/aivyx/releases/latest/download/aivyx-channel-installer.sh \
  | sh
aivyx --version
# aivyx 0.1.0
```

For a specific version, replace `latest` with the tag:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/AivyxDev/aivyx/releases/download/v0.1.0/aivyx-channel-installer.sh \
  | sh
```

### macOS first launch: Gatekeeper

Phase 61's release will not include code signing or notarization.
macOS quarantines unsigned binaries downloaded from the network.
Two ways past the warning:

**(a) Strip the quarantine attribute** (one-shot, recommended):

```sh
xattr -d com.apple.quarantine "$(command -v aivyx)"
```

**(b) Right-click → Open** the binary once from Finder. macOS
asks for confirmation; after that, future invocations work.

Signing + notarization is on the deferred-distribution list. It
requires an Apple Developer account and a CI-side cert pipeline;
it lands in a follow-up phase once operator pressure surfaces.

## Where files land

| File | Default location | Configurable? |
|---|---|---|
| `aivyx` binary | `~/.cargo/bin/aivyx` | Yes — `--install-path` flag on the installer |
| Config | `./aivyx.toml` (CWD) or `~/.config/aivyx/aivyx.toml` | Yes — `--config <path>` on `aivyx`; the wizard writes to CWD by default |
| Encrypted store | per-config (`[storage] path`) | Yes — TOML `[storage] path` |
| Daemon socket | `$XDG_RUNTIME_DIR/aivyx.sock` (Linux) / `$TMPDIR/aivyx.sock` (macOS) | No |
| Daemon PID file | `$XDG_RUNTIME_DIR/aivyx.pid` (Linux) / `$TMPDIR/aivyx.pid` (macOS) | No |
| Web UI port | `127.0.0.1:7843` | Yes — TOML `[daemon] web_ui_port` or `--web-ui-port <N>` |

## First-run checklist

After install:

1. **`aivyx init`** — interactive wizard. Detects Ollama at
   `http://127.0.0.1:11434` and offers it as the default
   provider (no API key required). Otherwise prompts for an
   Anthropic or OpenAI key. Writes `aivyx.toml` to your CWD with
   `0600` permissions. Three optional Profile prompts (assistant
   name, primary use case, communication style) seed your
   operator identity layer.

   **Phase 104 — verify-before-write.** When the operator picks
   Anthropic or OpenAI, the wizard hits the provider's
   `GET /v1/models` with the supplied key before writing
   `aivyx.toml` and confirms the chosen model is in the
   returned list. A wrong key or typo'd model is caught here
   and re-prompts the implicated field; a broken config never
   lands on disk. After three failed attempts the wizard
   offers a `Write anyway?` escape hatch — verify is a
   guardrail, not a lock. The Ollama path is already
   verified-by-existence through the wizard's `/api/tags`
   listing (an empty list prints a `Try: ollama pull
   llama3.2:3b` starter suggestion).

   **Faster path with a starter template** (Phase 66):
   `aivyx init --list-templates` to discover available starters
   (`coder`, `researcher`, `personal`), then
   `aivyx init --template <name>` to run the wizard with
   pre-filled defaults from the template. The generated
   `aivyx.toml` includes the template's role declarations, MCP
   blocks, and commented-out automation hints. See
   [`docs/TEMPLATES.md`](TEMPLATES.md) for the full template
   reference.

   **Authoring your own tool process** (Phase 103): if you want
   to ship a tool the substrate doesn't already include, run
   `aivyx tool init <path>` to scaffold a runnable Rust
   tool-process starter at `<path>` — `Cargo.toml`, a
   `src/main.rs` with the handshake + invocation loop, a
   `README.md`, and a conformance test. Edit the body of
   `handle_invocation`, build, then point a `[[tool_process]]`
   entry in `aivyx.toml` at the resulting binary. See
   [`docs/TOOL_SDK.md`](TOOL_SDK.md) for the full protocol.

2. **`aivyx`** — auto-spawns the daemon (foreground or
   background depending on flag), drops you into a REPL session,
   and serves the Web UI on `127.0.0.1:7843` if you enabled it.

3. **Visit `http://127.0.0.1:7843/`** for the Web UI: Chat,
   Missions, Audit (with cold-verify), Sessions, Profile,
   Persona tabs.

4. **`aivyx --verify-only`** at any time runs the offline
   HMAC audit-chain verification pass.

5. **`aivyx audit export`** (Phase 105) dumps the audit chain
   as JSONL on stdout. `--from <seq>` and `--limit <N>` slice
   the output via the same `entries_range` reader the Web UI
   Audit tab uses. Each line carries the full `SignedEntry`
   projection (seq, appended_at_ms, prev_mac, mac, event) so
   the export is re-verifiable downstream given a separately-
   supplied genesis seed. Offline-only — requires the
   operator's passphrase, cannot be triggered remotely over
   the daemon socket. See [`docs/AUDIT_EXPORT.md`](AUDIT_EXPORT.md)
   for the full reference + worked `jq` examples.

**Phase 110 — Skills Auto-Creation (Reflection Staging).**
Skills are procedural patterns the agent drafts after complex
turns and the operator approves through the existing
persona-proposal surface (`aivyx persona proposals`). Approved
skills land in the Persona chain as `LearnedSkill` deltas,
render into the agent's system prompt as a `## Learned skills`
section (one bullet per skill: `name: trigger`), and are
callable through `skills.list` (enumerate `{name, trigger}`)
and `skills.invoke` (read the full procedure body on demand).
Three new capability scopes — `skills.propose`, `skills.list`,
`skills.invoke` — all in `CEILING_TRUSTED`. Roles that should
draft + use skills declare these in their `capability_scopes`
alongside `persona.propose`. The agent-side auto-proposer
heuristic (fire reflection-cron-style after complex turns)
deferred to a follow-on phase; today the agent proposes
skills only when explicitly invoked through `reflection.propose`
with a `LearnedSkill` delta in the `persona_deltas` array.

**Phase 109 — Three new substrate tools (Amendment A12).**
P10's substrate tool count grew from ten to thirteen with
`git.status`, `git.diff`, and `net.dns`. The git tools are
read-only inspection of operator-configured repos; they share
a `git.read` capability scope qualified by repo path and shell
out to the system `git` binary (no Rust deps). To enable, add
a `[git]` section to `aivyx.toml` listing the allowed repo
paths:

```toml
[git]
repos = [
    "/home/me/projects/aivyx",
    "/home/me/projects/some-other-repo",
]
```

Each path is canonicalized at startup and must be a directory
containing a `.git/` entry — config errors surface at startup,
not at tool-call time. Without `[git]`, the git tools simply
don't register (zero-config posture; agents see no git tools
in their dispatch surface).

`net.dns` is unconditionally registered (no config required;
uses the existing `net.dns` scope base from Phase 0). Takes a
plain hostname (no scheme, no port, no slash) and returns the
resolved IP addresses.

6. **`aivyx mcp recipes`** (Phase 106) lists Aivyx's curated
   catalog of MCP servers worth enabling — `filesystem`,
   `github`, `gitlab`, `sqlite`, `postgres`, `time`, `fetch`,
   `brave-search`, `slack`, `memory`, `puppeteer`,
   `everything`. Bare form prints the list; `aivyx mcp
   recipes <name>` prints a paste-able `[[mcp_server]]` block
   plus an inline `[mcp_server.sandbox]` block so a
   copy-paste produces a sandboxed config (Phase 55 substrate
   posture). The canonical reference lives in
   [`docs/MCP_RECIPES.md`](MCP_RECIPES.md). Distinct from
   `aivyx mcp-server <name>` (Phase 46) which *runs* a
   bundled server — recipes is the catalog of external ones.

For deployment guidance (threat model, what Aivyx defends
against, what it doesn't), read
[`docs/THREAT_MODEL.md`](THREAT_MODEL.md) before exposing the
agent to anything sensitive.

## Running Aivyx on Discord (Phase 107)

The Discord adapter mirrors the Telegram pattern: one bot
account, configured per-operator, sees DMs and any guild
channels you've added the bot to. Same `SemiTrusted` tier
ceiling, same `/cancel` mid-turn handling, same Profile +
Persona + mission-gate behavior.

1. **Create a bot account** at
   [https://discord.com/developers/applications](https://discord.com/developers/applications).
   Bot → "Add Bot" → save the **token** (you'll only see it
   once; copy it somewhere safe).
2. **Enable required intents** under Bot → "Privileged Gateway
   Intents":
   - `MESSAGE CONTENT INTENT` — required (the bot needs to
     read message text). Discord gates this behind a developer-
     portal toggle; for a private bot in < 100 servers, just
     flip it on.
3. **Invite the bot** to a server (OAuth2 → URL Generator →
   scopes `bot` + permissions `Send Messages`, `Read Message
   History`). Or just DM the bot from the developer-portal
   account.
4. **Configure aivyx** — set the token via env or TOML:

   ```sh
   export AIVYX_DISCORD_TOKEN='your_bot_token_here'
   aivyx --channel discord
   ```

   …or in `aivyx.toml`:

   ```toml
   [discord]
   token = "your_bot_token_here"
   # application_id = 12345...  # Reserved for slash commands; not used in v1.
   ```

5. **Talk to the bot** — open a DM, type a message, watch the
   agent reply. Each Discord channel id gets its own memory
   partition (the same multi-tenant story Telegram's
   `chat_id`-keyed partitions provide), so DMs and guild
   channels stay isolated.

**Daemon-mode `/approve` / `/reject` text-command routing**
landed at Phase 111 (Adapter Production Wiring) alongside
the Discord daemon-frontend. In-process and daemon-mode
both resolve gates through the bot reply on Phase 111+.

## Running Aivyx on Slack (Phase 108)

The Slack adapter follows the same shape as Discord and
Telegram: one Slack app, one Socket Mode WebSocket from
aivyx to Slack, the bot sees DMs and channels it's been
invited to. SemiTrusted tier; per-`(team_id, channel_id)`
memory partitioning so a Slack bot installed in two
workspaces partitions cleanly even when channel ids
collide.

1. **Create a Slack app** at
   [https://api.slack.com/apps](https://api.slack.com/apps).
   "From scratch" → name your app → pick the workspace.
2. **Enable Socket Mode** under app settings → Socket Mode
   → toggle on. This will prompt you to create an
   **app-level token** with `connections:write` scope.
   Save the resulting `xapp-...` token.
3. **Configure bot scopes** under OAuth & Permissions →
   add `chat:write`, `channels:history`, `groups:history`,
   `im:history`, `mpim:history`, and `app_mentions:read`.
4. **Subscribe to events** under Event Subscriptions →
   bot events → `message.channels`, `message.im`,
   `message.mpim`, `message.groups`. Subscribe to the events
   relevant for where you want the bot to listen.
5. **Install to workspace** under Install App → save the
   resulting `xoxb-...` bot token.
6. **Invite the bot** to any channel you want it to listen
   in. DMs work out of the box.
7. **Configure aivyx** — set both tokens via env or TOML:

   ```sh
   export AIVYX_SLACK_BOT_TOKEN='xoxb-...'
   export AIVYX_SLACK_APP_TOKEN='xapp-...'
   aivyx --channel slack
   ```

   …or in `aivyx.toml`:

   ```toml
   [slack]
   bot_token = "xoxb-..."
   app_token = "xapp-..."
   # team_id = "T0123456789"  # optional: constrain to one workspace
   ```

8. **Talk to the bot** — open a DM with the bot or mention
   it in an invited channel. Each `(team_id, channel_id)`
   gets its own memory partition (multi-workspace bots
   partition cleanly even on colliding channel ids).

**Production state — Phase 111 closed the Socket Mode
live-wiring carve-out.** The `SlackMorphismTransport` Phase
108 stub is replaced with the production
`SlackClientEventsUserState` callback wiring; all five
adapters (Local + Telegram + Web UI + Discord + Slack) are
production-ready in-process AND daemon-mode after Phase
111. Live-bot smoke testing across the matrix is the
Channel Activation Milestone's job (operator-driven
verification pass, separate from the phase sequence).

## Persona auto-proposer (Phases 112-115)

The Persona auto-proposer is the optional self-learning
loop that drafts reusable Persona refinements from complex
turns. Phase 112 shipped the substrate (skills-only). Phase
113 made it operator-configurable through TOML. Phase 114
generalized it across the full 11-category PersonaDelta
surface — the agent now self-learns at every Persona axis,
not just at the skill layer.

The Phase 113 `[skills.auto_propose]` section stays as an
alias for `[persona.auto_propose.learned_skill]` —
pre-Phase-114 configs continue to work byte-identically.
New operators use the Phase 114 section:

```toml
[persona.auto_propose]
enabled = true
# Defaults are tuned for "fires on multi-tool work, skips
# chit-chat." Operator only needs `enabled = true` to opt
# into the loop.
# judge_model = "claude-haiku-4-5"
# judge_max_tokens = 800
# fuzzy_match_threshold = 0.80  # LearnedSkill dedup only

[persona.auto_propose.heuristic]
# tool_call_count_min = 3
# distinct_tool_id_min = 2
# duration_ms_min = 5000
# require_gate_resolve = false
# mode = "any"             # "any" or "all"

# Per-category configuration. Defaults: scalars OFF (each set
# replaces the previous value; high-stakes), lists ON (additive).
# Operator opts in to the scalar categories explicitly.

[persona.auto_propose.learned_skill]
# enabled = true
# auto_accept_confidence_threshold = 0.85

[persona.auto_propose.behavioral_preferences]
# enabled = true
# auto_accept_confidence_threshold = 0.85

# ... other list categories: behavioral_constraints,
# learned_context, communication_adaptations, character_traits,
# relationship_milestones, primary_use_cases ...

[persona.auto_propose.assistant_name]
# enabled = false                          # default OFF; high-stakes scalar
# auto_accept_confidence_threshold = 0.99  # require near-certainty

[persona.auto_propose.operator_profile]
# enabled = false
# auto_accept_confidence_threshold = 0.99

[persona.auto_propose.communication_style]
# enabled = false
# auto_accept_confidence_threshold = 0.99
```

After the section is configured, every `TurnOutcome::
Completed` fires a background-task auto-proposer pipeline:
a cheap heuristic gate filters candidates; the LLM judge
picks the right category and drafts the proposal in the
shape that category expects (LearnedSkill, ListAppend, or
ScalarSet); high-confidence non-dup proposals for enabled
categories auto-accept into the Persona chain; below-
threshold verdicts stage as Pending proposals the operator
resolves through `aivyx persona proposals approve`.

**Inspection flags** (Phase 113, generalized in Phase 114):
- `aivyx persona list --auto-only` shows ALL entries the
  auto-proposer wrote across every category (delta_id
  prefix `pd-auto-`).
- `aivyx persona list --manual-only` shows the complement.
- `aivyx audit export --event-type SkillAutoProposal`
  emits only the auto-proposer's audit-event variants for
  forensic walks (`jq`-able JSONL). Phase 114 entries
  carry the `category` field so operators can filter by
  category downstream.

**Self-correction loop (Phase 115).** The auto-proposer
also fires from FAILED turns (not just completed turns)
when `from_failed_turns = true`. The agent observes a
failure and proposes a Persona refinement that would
prevent recurrence — typically a BehavioralConstraint
("never X") or LearnedContext ("remember Y").

```toml
[persona.auto_propose]
enabled = true
from_failed_turns = true        # default false; opt in

[persona.auto_propose.failure_outcomes]
# Default: failed=true, timed_out=true, cancelled=false,
# escalated=false. Tune per failure-type.
# failed = true
# timed_out = true
# cancelled = false
# escalated = false
```

Per-failure-outcome enables let the operator be
conservative on operator-driven cancellations / agent
escalations (where the agent did the right thing under
D1's Tier-2 rules) while still learning from clear
failures. The `--event-type SkillAutoProposal` filter
includes a `source` field that distinguishes
`CompletedTurn` from `FailedTurn { failure_kind }` for
forensic separation.

**Escape hatches:**
- The auto-proposer never blocks a turn — failure-isolated
  background spawn. The user's reply is sent first; the
  pipeline runs after.
- Auto-accepted Persona deltas are revertible through the
  existing Phase 60 surface: `aivyx persona revert
  <delta_id>`. The revert is itself an audit-chained chain
  append, so the forensic trail stays intact.
- Per-category enable flags let the operator opt out of
  specific axes (e.g. keep `learned_skill` on but
  `behavioral_preferences` off) without disabling the
  whole loop.
- Per-failure-outcome flags let the operator scope which
  failure types fire self-correction (Phase 115).
- The whole feature can be disabled by setting top-level
  `enabled = false` (or removing the section). The auto-
  proposer bypass costs zero — no LLM call, no chain
  write.

## Tool/skill relevance hints (Phases 116-117)

The Phase 116 relevance ledger tracks per-tool and
per-skill success/failure outcomes per keyword-extracted
turn pattern. After this phase, the agent's tool/skill
selection — historically pure LLM intuition — can be
augmented by observed historical outcomes the operator
can inspect and tune.

Off by default; enable the section to opt in:

```toml
[tool_relevance]
enabled = true
# Defaults are tuned for cheap-deterministic operation:
# zero LLM cost per turn, bounded prompt-section size.
# max_keywords = 5         # top-K longest non-stopword tokens
# min_outcomes_to_show = 2 # don't show one-data-point rows
# top_k_per_section = 5    # max rows per Tools / Skills subsection
```

After the section is configured, the daemon's post-finalize
hook records each turn's tool outcomes against the user
input's keyword key (Q1a: lowercased, stopword-filtered,
length-ordered top-K tokens, lex-sorted, pipe-joined for
storage). On the next turn with a matching keyword key,
the substrate is ready to render a `## Tools recently used
for similar tasks` section augmenting the LLM's picks.

**Section format** (when the renderer is hooked into the
live prompt path):

```
## Tools recently used for similar tasks

Based on keywords: code, rust

Tools:
- memory.read: 5 successes, 0 failures
- web.fetch: 3 successes, 1 failure

Skills:
- research-topic: 2 invocations (2 successes, 0 failures)
```

**Phase 117 closes both Phase-116-internal deferrals.**
The relevance section now reaches the LLM in live turns
via a `RelevancePromptRefiner` that plugs into the Phase
79 `SystemPromptRefiner` slot on the planner's config; if
Phase 79 adaptive Persona is also armed, both refiners
chain in a single install (Phase 79 inner, Phase 117
outer, composing as `base + adaptive Persona + relevance
section`).

Per-skill tracking lands via a new
`AuditEvent::SkillInvocation` variant that `skills.invoke`
emits alongside its regular `ToolCall` audit entry. The
ToolCall keeps the input-hash (D4 secrets-safety
preserved); the SkillInvocation carries the skill name in
cleartext so Phase 116's `record_turn_outcomes` can
populate per-skill ledger rows. `aivyx audit export
--event-type SkillInvocation` filters to the new variant
for forensic walks.

**Escape hatches:**
- The recording hook never blocks a turn — detached
  `tokio::spawn` after finalize. Audit-walk failures log
  WARN and don't affect the turn.
- Operator can inspect the encrypted ledger via a future
  `aivyx tool-relevance dump` CLI (deferred — until the
  live-prompt path lands, the prompt section IS the
  inspection surface).
- Disabling the section (or setting `enabled = false`)
  bypasses the substrate entirely. The Phase 116 ledger
  domain stays present in storage but no rows are
  written or read.

## Profile/Role refinement (Phase 118)

Phase 118 closes Chapter E with the last named axis:
**outcome-driven Profile/Role refinement**. The agent
observes recurring task shapes that don't fit the current
operator-declared Profile or operator-curated Role config
and proposes refinements as `ProfileHint` or
`RoleDefinitionSuggestion` Persona-chain entries. The
operator reviews each proposal and copies the rendered
draft into `aivyx.toml` if they want to act on it.

**Contract preservation.** Phase 118 honors:
- **P13** (Profile is operator-declared, Phase 56 amendment).
  `ProfileHint` proposals NEVER mutate `aivyx.toml`. Approved
  hints sit in the Persona chain as a record-of-suggestion
  the operator can read at their convenience.
- **P9** (Per-Role full capability declaration, Phase 13).
  `RoleDefinitionSuggestion` proposals NEVER mutate
  `aivyx.toml`. Approved drafts likewise sit in the chain
  for operator copy-paste.

Both categories are **always-staged for operator approval**,
hard-coded at the routing layer regardless of judge
confidence. Operators who don't want auto-proposing these
categories at all can disable them via TOML.

**TOML config sub-sections:**

```toml
[persona.auto_propose.profile_hint]
enabled = true          # default; set false to silence
# auto_accept_confidence_threshold parses but is
# IGNORED at runtime — the always-staged routing
# override forces Staged regardless. Documented here
# for type-shape consistency only.

[persona.auto_propose.role_definition_suggestion]
enabled = true          # default; set false to silence
```

**Operator workflow:**

1. The auto-proposer fires after a turn whose signals
   cross the heuristic gate. Two new Phase 118 heuristic
   signals feed this:
   - `profile_pattern_repeated` — fires when the current
     turn's keyword_key (Phase 116) has accumulated
     ≥ `profile_pattern_recurrence_min` (default 5)
     prior outcomes in the relevance ledger.
   - `role_shape_recurring` — fires when the recent
     session window contains ≥
     `role_shape_scope_denied_min` (default 2)
     `ScopeDenied` audit events.
2. The LLM judge picks `ProfileHint` or
   `RoleDefinitionSuggestion` and drafts the payload
   inline (field + suggested_value + rationale, or full
   role draft + rationale). The judge is instructed to err
   on the side of EXPLICIT rationales because the operator
   reads them.
3. `decide_routing` forces `Staged` regardless of
   confidence. The proposal lands in the persona-proposal
   chain as Pending, and a `SkillAutoProposal` audit event
   records `outcome=Staged` + `category=ProfileHint` (or
   `RoleDefinitionSuggestion`) for forensic visibility.
4. The operator reviews:

   ```sh
   aivyx persona proposals list
   # [Pending] pp-abc...  category=ProfileHint  ...
   #   op = {"kind":"AppendList","value":"..."}

   aivyx persona proposals show pp-abc...
   # Proposal pp-abc...
   # =========================
   #   category    = ProfileHint
   #   proposed op:  { ... raw JSON ... }
   #   rendered draft:
   #     field            = communication_style
   #     suggested_value  = "terse and bullet-formatted"
   #     rationale        =
   #       operator consistently uses bullets in their
   #       own messages and asks for shorter replies
   #
   #   To apply: edit aivyx.toml [profile] and update the
   #   field above. Phase 118 does NOT auto-mutate aivyx.toml.
   ```
5. **Phase 119 — apply the hint with one command.** From
   Phase 119 onward, the operator doesn't have to translate
   the rendered draft into a TOML edit by hand. Approve the
   proposal first, then run the apply-helper:

   ```sh
   aivyx persona proposals approve pp-abc...
   aivyx profile apply-hint pp-abc...
   # Apply `communication_style` = "terse and bullet-formatted" to aivyx.toml?
   # [y/N] (re-run with --yes to skip this prompt)
   y
   #
   # Applied `communication_style` to aivyx.toml.
   # Audit event `ProfileHintApplied` recorded for proposal `pp-abc...`.
   # Restart the daemon for the new value to take effect:
   # `aivyx daemon stop && aivyx`.
   ```

   The apply is atomic (tmp-file + rename); comments and
   other sections in `aivyx.toml` are preserved
   byte-for-byte. List-field hints (e.g.
   `behavioral_preferences`) append idempotently; re-running
   the same apply twice is a no-op.

6. For a `RoleDefinitionSuggestion`, the analogous Phase 119
   command is `aivyx role import`:

   ```sh
   aivyx persona proposals approve pp-role-xyz...
   aivyx role import pp-role-xyz...
   # Import role `research-deploy` inheriting from `research` into aivyx.toml?
   # [y/N] (re-run with --yes to skip this prompt)
   y
   #
   # Imported role `research-deploy` into aivyx.toml.
   # Audit event `RoleDraftImported` recorded for proposal `pp-role-xyz...`.
   # Restart the daemon for the new role to take effect:
   # `aivyx daemon stop && aivyx`.
   ```

   Refuses to overwrite an existing `[roles.<name>]`
   section without `--force`. With `--force`, replaces the
   section entirely.

7. Either way, `aivyx persona proposals approve pp-abc...`
   marks the chain entry as accepted (or `reject` to
   discard). Approved proposals land in
   `EffectivePersona::profile_hints` /
   `EffectivePersona::role_drafts` as a record-of-decision;
   the operator can list them later with the same `list`
   command (status `Applied`).

**Escape hatches:**
- Set `enabled = false` on either sub-section to silence
  proposing entirely. The heuristic still fires and the
  judge still runs for OTHER categories; only the Phase
  118 categories drop with `DroppedCategoryDisabled`.
- Set both `enabled = false` AND disable the Phase 116
  relevance ledger to suppress the `profile_pattern_repeated`
  signal source. The `role_shape_recurring` signal sources
  directly from the audit chain and stays active.
- Phase 118 never auto-mutates `aivyx.toml`. The operator
  is always in the loop. If a `ProfileHint` or `RoleDraft`
  approval shows up that the operator doesn't want to act
  on, the approval is a no-op against the live config —
  the entry sits in the chain as "noted but not applied"
  state.
- **Phase 119 — apply commands also never auto-mutate
  without operator action.** `aivyx profile apply-hint` and
  `aivyx role import` are explicit operator gestures. They
  confirm with `[y/N]` by default; pass `--yes` to skip
  the prompt in scripted workflows. The apply step records
  a `ProfileHintApplied` / `RoleDraftImported` audit event
  via daemon IPC; if the audit-record step fails after the
  file mutation lands, the CLI surfaces a soft warning and
  the operator can re-run the command to re-record (the
  TOML edit is idempotent).

## Inspecting the tool-relevance ledger (Phase 119)

The Phase 116 `KeyDomain::ToolRelevanceLedger` is AEAD-
encrypted at rest, so before Phase 119 operators had no
read path into the per-keyword-key outcome rows the self-
learning loop had accumulated. Phase 119 closes that
deferred surface:

```sh
aivyx tool-relevance dump
# keyword_key      surface  identifier         success  failure  last_seen_unix_ms
# ---------------  -------  -----------------  -------  -------  -----------------
# research+deploy  skill    summarize-pdf            3        0      1715000040000
# research+deploy  tool     fs.read                  7        1      1715000060000
# research+deploy  tool     web.fetch                2        0      1715000050000
# (ledger empty — no per-keyword-key outcomes recorded yet)  ← if empty
```

Rows are sorted ascending by `(keyword_key, surface,
identifier)` for stable terminal scanning. Column widths
size to the longest value — keyword keys never truncate.
Restrict the dump to a single keyword key with
`--keyword-key`:

```sh
aivyx tool-relevance dump --keyword-key research+deploy
```

The dump talks to the running daemon over IPC; it requires
the daemon to be up. With `[tool_relevance] enabled =
false` in `aivyx.toml`, the dump errors with
`no_tool_relevance_ledger` rather than returning an empty
table (the substrate is bypassed entirely, not silently
empty).

## Local-LLM tool-call recovery (Phase 120)

Local models like qwen3.6:27b and gemma4:31b occasionally
hallucinate tool names — emitting `fs_read` when the
registered tool is `fs.read`, or `web_fetch` instead of
`web.fetch`. Cloud models (Anthropic) rarely do this;
local models with smaller training corpora are the
dominant source.

Before Phase 120, hallucinated names caused turns to fail
ungracefully: the planner couldn't dispatch a non-existent
tool, and the agent terminated with
`TurnOutcome::Failed`. Phase 120 closes that failure mode
with belt-and-suspenders validation at the LLM-provider
boundary AND fuzzy-match recovery at the planner.

**The fix is substrate-shaped, not model-shaped.** We
can't make local models stop hallucinating; we catch the
hallucination at the boundary and give the model a
structured response that lets it recover.

### What happens when the model hallucinates

1. The OpenAI/Ollama or Anthropic provider classifies
   every emitted tool name against the canonical tool set
   the request advertised. Unknown names get flagged as
   `NameResolution::Unknown { original }` before the
   stream terminates.

2. The planner's recovery path computes Phase 112's
   `title_similarity` (tokenized Jaccard) against every
   registered tool. The algorithm normalizes separators
   and case, so `fs_read` and `fs.read` both tokenize to
   `{fs, read}` — Jaccard 1.0.

3. **Above the operator-configured threshold (default
   0.80)**, the planner dispatches the matched tool and
   records the verbatim original name in the audit chain
   via `AuditEvent::ToolCall.auto_corrected_from`.
   Operator forensics see the auto-correction explicitly:

   ```sh
   aivyx audit export --event-type ToolCall | \
     jq 'select(.auto_corrected_from)'
   # {
   #   "kind": "ToolCall",
   #   "tool_id": "fs.read",
   #   "auto_corrected_from": "fs_read",
   #   ...
   # }
   ```

   Rates of `Some(_)` entries across a window of audit
   events are a useful diagnostic when picking between
   local models — qwen3.6:27b with N auto-corrections per
   100 ToolCalls vs gemma4:31b with M tells you which
   model has the cleaner tool-call protocol.

4. **Below threshold**, the planner emits a synthetic
   `unknown_tool` tool-result back to the model with a
   structured "did you mean?" body:

   ```json
   {
     "error": "unknown_tool",
     "message": "tool 'do_the_thing' is not registered. Did you mean 'fs.read', 'fs.write', 'memory.read'?",
     "did_you_mean": ["fs.read", "fs.write", "memory.read"]
   }
   ```

   The top-3 suggestions are ranked by `title_similarity`
   descending. The model can parse the `did_you_mean`
   array on its next turn and retry with the right name.

### Operator config knob

The fuzzy threshold is operator-configurable via
`aivyx.toml`:

```toml
[providers]
tool_name_auto_correct_threshold = 0.80   # default
```

Float in `[0.0, 1.0]` — out-of-range values reject at
TOML-parse time with `ConfigError::Invalid`. The threshold
is operator-conservative-leaning at the default:

- `0.80` (default) — matches Phase 112's fuzzy default.
  Catches the `fs_read` / `web_fetch` / `git_status`
  separator-hallucination patterns the project memory
  documents qwen3.6 emitting.
- `1.0` — exact match only. Disables fuzzy recovery
  entirely; any Unknown name falls through to the
  synthetic error path. Operator-paranoid posture: never
  trust the planner to pick the model's intent.
- `0.65–0.75` — more aggressive recovery. Useful for
  smaller local models with messier tool-call
  protocols. Watch the audit-export
  `auto_corrected_from` count to confirm the lowered
  threshold isn't mis-dispatching unrelated tools.
- `0.0` — every match clears (auto-corrects to the
  first registered tool). Not useful in practice; pin
  the inclusive-bound semantics rather than enabling
  garbage-out behavior.

### What this does NOT change

- **Cloud-model behavior is unchanged in practice.**
  Anthropic and OpenAI cloud models rarely emit
  hallucinated tool names; the provider-side validation
  fires but classifies every call as `Known`, the
  planner dispatches directly, and `auto_corrected_from`
  stays `None`. Operators paying for cloud inference see
  no behavioral difference.
- **The audit chain stays wire-compatible.**
  `auto_corrected_from: None` serializes WITHOUT the
  field (`#[serde(default, skip_serializing_if =
  "Option::is_none")]`) — pre-Phase-120 chain entries
  decode unchanged, and a Phase 120 read of a Phase 119
  ToolCall produces byte-identical canonical JSON. HMAC-
  chain integrity preserved.
- **No new tool added to the P10 substrate.** Phase 120
  ships substrate that fixes the existing tool-dispatch
  path; the 13-tool substrate cap stays at thirteen.

### Escape hatches

- Set `tool_name_auto_correct_threshold = 1.0` in
  `aivyx.toml` to disable fuzzy recovery. The provider
  still classifies, but the planner never auto-corrects;
  every Unknown name produces the `unknown_tool` error
  path immediately.
- The provider-side validation always runs; there is no
  knob to disable it. Cheap pure-function check; no LLM
  cost.
- Audit forensics: `aivyx audit export --event-type
  ToolCall | jq 'select(.auto_corrected_from)' | jq -s
  length` counts auto-corrections in the chain. Use this
  to evaluate whether your local-model choice is
  producing too much noise (and consider raising the
  threshold or switching to a model with a cleaner
  protocol).

## Native Ollama provider (Phase 121)

Phase 25 added OpenAI-compatible LLM support; Phase 34
brought Ollama to first-class status by routing
`provider = "ollama"` through that same OpenAI-compat
path. The translation worked but lost fidelity on
Ollama-specific options (`num_ctx`, `num_predict`,
`mirostat`) and on Ollama's native JSONL streaming
protocol.

**Phase 121 ships a dedicated `OllamaProvider`** that talks
Ollama's `/api/chat` natively. After Phase 121, `provider
= "ollama"` in `aivyx.toml` routes to the native adapter
**transparently** — operators using Ollama get native
benefits without changing their config.

### What changed for `provider = "ollama"`

- **Endpoint**: `/api/chat` (was `/v1/chat/completions`
  through the OpenAI-compat path).
- **Streaming**: native JSONL (newline-delimited JSON
  objects) instead of SSE `data:` framing.
- **Tool calls**: arrive complete in the final `done:
  true` chunk (Ollama's actual protocol; the OpenAI-compat
  path was reassembling delta-streamed arguments that
  Ollama never sent that way).
- **Usage**: `prompt_eval_count` → input tokens,
  `eval_count` → output tokens, on the terminal chunk.
- **Tool-call arguments**: passed as JSON **objects** on
  the wire (Ollama's native format), not JSON-encoded
  strings.

**Behavior NOT changed:**
- Existing `aivyx.toml` files with `provider = "ollama"`
  work unchanged. The base-URL handling, model-name
  selection, and channel adapters all continue to work.
- Phase 120's tool-call recovery substrate flows uniformly
  through the native adapter: `NameResolution::Unknown`
  classification fires on hallucinated names (e.g.
  qwen3.6:27b emitting `fs_read` when the registered
  tool is `fs.read`), and the planner's fuzzy-match
  recovery dispatches as before.
- The OpenAI-compat path stays for explicit `provider =
  "openai"` (cloud OpenAI or non-Ollama OpenAI-compat
  services).

### Configuring Ollama-specific options

Phase 121 introduces a new `[ollama]` section in
`aivyx.toml` for the operator-relevant subset of Ollama's
modelfile options. All fields are optional; unset fields
fall through to Ollama's per-model defaults.

```toml
provider = "ollama"
model = "qwen3.6:27b"

[ollama]
# Resource knobs:
num_ctx = 16384       # context window override (default: per-model)
num_predict = 2048    # max tokens to generate
num_thread = 8        # threads for the runtime

# Sampling knobs:
mirostat = 2          # 0 = off, 1 = Mirostat, 2 = Mirostat 2.0
top_k = 40
top_p = 0.9
repeat_penalty = 1.1
repeat_last_n = 64

# Reproducibility:
seed = 42
```

These propagate into Ollama's request `options: {...}`
block. Ollama's per-model defaults apply for any field
the operator hasn't overridden — `num_ctx`, in
particular, varies widely by model (some are 2048, some
are 128K+).

### Operator-protected Ollama deployments

Vanilla `ollama serve` doesn't require authentication, but
operators running Ollama behind a reverse proxy (Caddy,
nginx, Cloudflare Access) can attach an API key. The
provider emits `Authorization: Bearer <key>` only when
`OLLAMA_API_KEY` is set; absent means no header (matches
the OpenAI provider's defensive empty-key posture).

```sh
export OLLAMA_API_KEY="opaque-token-issued-by-your-proxy"
aivyx
```

### When to pick `provider = "openai"` instead

The native adapter is the right default for any Ollama
deployment. The OpenAI-compat path is the right choice
when:

- You're targeting cloud OpenAI directly (Phase 25 use
  case).
- You're targeting a non-Ollama OpenAI-compat service
  (vLLM with OpenAI-compat enabled, LM Studio,
  llama.cpp's `--api-base`, etc.) that doesn't speak
  Ollama's native JSONL protocol.

In both cases use `provider = "openai"` and set
`OPENAI_BASE_URL` to the target endpoint.

### Phase 120 substrate uniformity

The Phase 120 tool-name recovery substrate flows uniformly
through the native Ollama adapter. The provider classifies
each emitted tool name against the request's advertised
set; the planner runs `title_similarity` fuzzy-match
recovery above the operator-configured threshold; auto-
corrections land in `AuditEvent::ToolCall.auto_corrected_from`
with HMAC-chain-byte-identical canonical-JSON for the
dominant (no-correction) case. Same operator-forensics
recipe works:

```sh
aivyx audit export --event-type ToolCall | \
  jq 'select(.auto_corrected_from)'
```

### Honest scope caveat carried from open doc

The Phase 121 open doc surfaced this at sign-off:
**the native Ollama adapter does not fix model-shaped
hallucination patterns directly.** A model that emits
`fs_read` will keep emitting `fs_read`. What Phase 121
gives is the substrate gain: native protocol fidelity,
operator-tunable Ollama options, and no OpenAI-compat
translation layer to debug. The hallucination recovery
itself is Phase 120's substrate, running uniformly
through both adapter paths.

## Per-model prompt variants (Phase 122)

After Phase 121 shipped, real-use signal across 13
interactive turns produced **zero tool calls** between
qwen3.6:27b and gemma4:31b. Both models confabulate
their tool catalogs at the prose level (qwen3.6
invented "Good Morning" as a tool; gemma4 invented
60+ entirely-fictional tools) and refuse or return
empty when commanded to invoke a tool by exact name.
Phase 120's substrate is orthogonal to this failure
mode — it catches hallucinated *invocations*; this is
hallucinated *capability denial*.

**Phase 122 ships structured per-turn tool-catalog
injection** with operator-tunable per-family
selection. The fix is prompt-side: tools flow through
Ollama's protocol `tools: [...]` array *and* land in
the assembled system prompt under a `## Tools
available` block. The catalog block forces the
protocol-array catalog into the model's visible prose
context where its prose-level reasoning cannot ignore
it.

### Per-family TOML override

The operator-overridable surface is a new sub-table
under `[ollama]`:

```toml
provider = "ollama"
model = "qwen3.6:27b"

[ollama.prompt_strategies]
qwen3 = "structured_injection"       # default for qwen3.x
gemma4 = "structured_injection"      # default for gemma4
llama3 = "none"                      # default for llama3.x
```

Each value is one of:
- `"none"` — pre-Phase-122 behavior. Tools flow only
  via the Ollama protocol `tools: [...]` array. The
  assembled system prompt is unchanged.
- `"structured_injection"` — append a `## Tools
  available` block listing every tool the active role
  can invoke by exact name (filtered by the role's
  `tool_allowlist`), with a one-line preamble
  discouraging invention.

Keys are family strings, not full model names. The
binary maps a model name to its family at startup:

| Model name           | Family   |
|----------------------|----------|
| `qwen3.6:27b`        | `qwen3`  |
| `qwen3.5:7b`         | `qwen3`  |
| `qwen2.5:7b`         | `qwen2`  |
| `gemma4:31b`         | `gemma4` |
| `gemma3:9b`          | `gemma3` |
| `llama3.1:latest`    | `llama3` |
| `llama2:13b`         | `llama2` |
| `claude-haiku-4-5`   | (none)   |

Anything that doesn't parse to a known Ollama family
prefix (cloud model names, future families this build
doesn't recognize) gets `OllamaFamilyStrategy::None`
unconditionally — operator-conservative: a new model
release doesn't silently get substrate it wasn't
tested against.

### Per-family defaults

Operators who don't set `[ollama.prompt_strategies]`
get pre-baked defaults from the Phase 122 sign-off
diagnostic data:

| Family   | Default                  | Why                                               |
|----------|--------------------------|---------------------------------------------------|
| `qwen3`  | `structured_injection`   | qwen3.6:27b confabulated tools, refused fs.write  |
| `gemma4` | `structured_injection`   | gemma4:31b confabulated 60+, returned empty       |
| `llama3` | `none`                   | tool-use protocol presumed reliable               |
| _other_  | `none`                   | conservative default for untested families        |

An explicit `[ollama.prompt_strategies] <family> =
"..."` always wins over the default.

### Startup banner provenance

The config banner shows which strategy resolved for
your model and where it came from:

```
aivyx config sources:
  provider          = ollama (toml)
  model             = "qwen3.6:27b" (toml)
  ollama_prompt_strategy = "structured_injection" (family: qwen3, default)
```

Provenance suffixes:
- `family: <name>, default` — model detected; no
  override in your TOML; per-family default applied.
- `family: <name>, override` — model detected; your
  `[ollama.prompt_strategies] <name>` override is
  being honored.
- `family: undetected` — model name didn't parse to
  any known family. Strategy always shows `"none"`.

### Cost: per-turn input-token overhead

The structured-injection block lists every tool the
active role can invoke. For the default role on a
typical install, that's ~12-20 tools at ~30-50
tokens each — roughly **400-700 input tokens per
turn** on top of the existing prompt. For small-
context models or cost-sensitive cloud deployments,
this trade may be unfavorable; the per-family
`none` override is the operator's escape hatch.

The block scales with the active role's
`tool_allowlist`: a role with a restricted allowlist
sees only the tools that survived the filter, not
the full registered tool set.

### Honest scope caveat carried from open doc

The Phase 122 open doc flagged two failure modes the
substrate might not fix:
- **gemma4's capability-denial prior may be
  prompt-unreachable.** If the model's training prior
  on "what AI assistants can do" dominates any in-
  prompt reinforcement, structured injection won't
  rescue it. Exit-doc will document the observed
  outcome regardless.
- **qwen3.6's verbal refusal may persist** even with
  the catalog block visible. Same reasoning: model
  prior may dominate.

Phase 6 Q5 honesty applies to RESULTS, not to
ANTICIPATION. Whatever the live verification at exit
shows, the exit doc reports it.

## Moving Aivyx to a new machine

Phase 64 ships **identity export**: a portable snapshot of your
operator-declared Profile and the reflection-approved Persona
chain. Useful for backup before risky changes, for migrating
between machines (laptop → VPS, old host → new host), or for
inspecting the chain offline with `jq`.

```sh
# On the source host (daemon must be running):
aivyx identity export ~/aivyx-snapshot.json
# Wrote N deltas + Profile to ~/aivyx-snapshot.json
# File permissions: 0600 (owner-only).
```

The file is pretty-printed JSON with the following shape:

```json
{
  "schema_version": 1,
  "exported_at": "2026-05-14T14:30:00Z",
  "source_host": "laptop.local",
  "profile": { "assistant_name": "...", "..." : "..." },
  "persona": {
    "deltas": [ { "seq": 0, "delta": { "..." : "..." } }, ... ],
    "effective_at_export": { "..." : "..." }
  }
}
```

The chain's HMAC MACs are deliberately omitted from the export
— the per-host HMAC key is not portable. On import the chain
is re-signed with the target host's key. Trust comes from
operator authority, not cross-host cryptographic provenance.

To restore a snapshot on a target host (Phase 65):

```sh
# On the target host (daemon must be running):
aivyx identity import ~/aivyx-snapshot.json

# If the target host already has a Persona chain, the import
# refuses by default to avoid silent overwrite. Pass --force
# to wipe and replace:
aivyx identity import ~/aivyx-snapshot.json --force
```

On success the daemon refreshes its runtime persona state
immediately — the next agent turn sees the imported persona
without a restart.

**Profile import is operator-driven.** The export bundle
includes the source host's `[profile]` section for reference,
but `aivyx identity import` does not auto-write `aivyx.toml`.
To apply the imported Profile, hand-edit the target host's
`aivyx.toml` to match the bundle's `profile` block, then
`aivyx daemon stop && aivyx` to reload. This keeps the
destructive-write scope tight to one on-disk artifact (the
encrypted Persona chain).

## Debugging missing notifications

When a scheduled briefing or trigger-fired notification doesn't
arrive, the audit chain has the answer. Phase 67 records every
auto-notify fire (delivered, skipped, or failed) as an
`AutoNotifyDispatched` entry alongside `TurnStarted` /
`TurnEnded`. To inspect:

```sh
# Walk the chain offline (no daemon needed):
aivyx --verify-only

# Or open the Web UI's Audit tab at http://127.0.0.1:7843
```

Filter on `AutoNotifyDispatched` and look at the `outcome`
field:

- `Delivered` — the notification reached the backend; if you
  didn't see it, check the backend (Telegram bot still
  authorized? webhook endpoint reachable from operator side?).
- `SkippedEmptyResponse` — the agent's turn produced no text;
  often a sign the trigger prompt was misconfigured or the
  model returned nothing useful.
- `Failed { error_kind, error_message }` — backend rejected
  the dispatch. `error_kind` is one of `transport`, `auth`,
  `rejected`, `timeout`, `unknown_target`.

Correlate by `session_id` to find the corresponding
`TurnStarted` / `TurnEnded` events for the same trigger fire.

## Multi-target + conditional dispatch (Phase 72)

Every trigger (`[[schedule]]`, `[[webhook]]`, `[[file_watch]]`)
now accepts three notify-flavoured knobs that compose with the
notify-target backends in the next sections.

**Multi-target fan-out** — `notify_targets = ["phone",
"desktop"]` fires every named target concurrently when the
trigger completes. One backend's transport failure doesn't
block the others; each per-target outcome audits independently
in the audit chain. The singular `notify_target = "phone"`
stays valid as a one-element alias.

**Default-target sugar** — Mark ONE `[[notify_target]]` block
with `default = true`. Triggers that omit `notify_targets`
fall through to it at config-load time. At most one default
is allowed.

**Conditional notify** — `notify_when` gates dispatch by turn
outcome:

| Value | Behavior |
|---|---|
| `"always"` (default) | Dispatch on every fire. Empty responses still get the Phase 63 `SkippedEmptyResponse` audit treatment. |
| `"on_failed"` | Dispatch only when the turn outcome is `Failed` or `TimedOut`. Useful for "ping me when my morning job breaks." |
| `"on_completed_non_empty"` | Dispatch only when the turn completed AND the rendered body is non-whitespace. The common shape for "stop pinging me on every cron fire — only when there's something to say." |

A condition-gated skip records
`AutoNotifyOutcomeSummary::SkippedByCondition { condition }`
in the audit chain so forensic searches can answer "why didn't
this fire?" definitively.

## Per-target retry, rate limit, history (Phase 73)

Each `[[notify_target]]` block accepts four optional fields
that tune backend behavior. Defaults preserve Phase 62
behavior — operators who don't set them get one attempt per
fire and no rate limiting.

**Retry** — flat per-target fields:

```toml
[[notify_target]]
name = "alerts"
kind = "webhook"
url = "https://ntfy.sh/aivyx-personal-2026"
retry_count = 5             # default 0, cap 10
retry_backoff_ms_start = 200  # default 500ms, min 100ms
```

Retries fire on `Transport`, `Timeout`, and `Rejected` with
HTTP status ≥ 500 — the transient-failure class. `Auth`,
`UnknownTarget`, and `Rejected` with status < 500 never
retry; those need operator intervention or are programmer
errors. Backoff is exponential: `backoff_ms_start * 2^attempt`.
For `retry_count = 5` starting at 200ms the schedule is
0ms (initial) + 200ms + 400ms + 800ms + 1.6s + 3.2s between
attempts — total worst-case latency about 6.2 seconds per fire.

**Rate limit** — in-memory sliding-window token bucket per
target:

```toml
[[notify_target]]
name = "phone"
kind = "telegram"
chat_id = "123456789"
rate_limit_max = 20
rate_limit_window_secs = 3600
```

Both fields must be set together or neither. Excess attempts
skip the backend call and record
`AutoNotifyOutcomeSummary::SkippedByRateLimit { limit,
window_secs }` in the audit chain. State lives in memory for
the daemon's lifetime — restart resets the bucket. v1 trades
durability for simplicity; the audit chain remains the
canonical record of what actually dispatched.

**History review** — operators have two surfaces:

```sh
# Terminal — flat-text table with seq, timestamp, target,
# outcome, trigger source/id, optional detail column.
aivyx notify history                          # latest 100, all targets
aivyx notify history --target phone           # filter by target
aivyx notify history --target phone --limit 500  # max per page
```

Web UI: open `http://127.0.0.1:7843/` and click the
**Notifications** tab. Per-target chips auto-populate from
the loaded page; outcome badges colour-code delivered (teal)
vs failed (orange) vs skipped (amber).

## Email notifications (Phase 68)

Most operators don't run a Telegram bot but everyone has email.
The `email` notify kind covers that. Add a `[[notify_target]]
kind = "email"` block + a top-level `[email]` section with
SMTP credentials, and the `notify.send` tool / trigger
auto-notify path both route through it.

**Quick setup by provider:**

- **Gmail / Google Workspace** — host `smtp.gmail.com`, port
  `587`, `tls_mode = "starttls"` (default). With 2FA enabled
  (which it should be), generate an app password at
  *Account → Security → App passwords* and use it as
  `[email] password`.
- **Fastmail** — host `smtp.fastmail.com`, port `587`. App
  password from *Settings → Privacy & Security → Integrations*.
- **ProtonMail** — run the ProtonMail Bridge locally; SMTP
  goes to `127.0.0.1` with the bridge-supplied credentials.
- **Self-hosted Postfix / Mailcow** — whatever your
  submission port is (usually 587), STARTTLS, plain
  username + password.
- **AWS SES** — host `email-smtp.<region>.amazonaws.com`,
  port `587`, IAM-derived SMTP credentials.

**TLS is mandatory.** Aivyx rejects `tls_mode = "none"` at
config-load time because PLAIN/LOGIN auth over cleartext
leaks credentials. If you need a plain-text relay for testing,
use a localhost SMTP capture tool instead.

**No OAuth2 yet.** Phase 68 ships PLAIN/LOGIN auth only.
Gmail/Office 365 users with strict workspace policies that
prohibit app passwords need to wait for the OAuth2 phase or
use a different provider in the meantime.

## Web UI desktop notifications (Phase 69)

The Web UI's localhost-only page at `127.0.0.1:7843` (Phase 39)
can deliver OS-level desktop notifications + an in-page toast
banner whenever the agent or trigger auto-notify fires.
Operators who already keep the Web UI tab open get the
lowest-friction notification path — no API keys, no SMTP
setup, no bot tokens.

**Enable it in two steps:**

1. Add a `[[notify_target]] kind = "web-ui"` block to
   `aivyx.toml` (and make sure the Web UI server is enabled —
   it ships on by default):

   ```toml
   [[notify_target]]
   name = "desktop"
   kind = "web-ui"
   ```

2. Open `http://127.0.0.1:7843/` in a browser. On first load a
   banner asks "Enable desktop notifications" — click *Enable*
   and grant the browser's permission prompt. The agent's
   `notify.send` tool and any trigger with
   `notify_target = "desktop"` will now reach you.

**The browser tab must be open.** Desktop notifications are
delivered over the existing WebSocket bridge; close the tab
and notifications stop firing for that target. Pair Web UI
notify with `kind = "email"` (or `kind = "telegram"`) when you
want notifications to land while you're away from the laptop —
the audit chain records every dispatch either way.

**One Web UI per daemon.** Multiple `kind = "web-ui"` targets
all funnel into the same browser fan-out, so naming them
differently only affects the per-target audit name; the
operator-visible behavior is identical.

## Memory subsystem (Phase 74)

Phase 74 completes the self-learning triad — Persona (P14),
reflection (Phases 70-71), and now a first-class memory
surface — with three operator knobs.

**Keyword search.** The agent gets a `memory.search` tool
(case-insensitive substring across topics + bodies, requires
the cross-topic `memory.read:topic:*` wildcard scope).
Operators have terminal + Web UI parity:

```sh
aivyx memory list                    # every topic
aivyx memory show <topic> [--limit N]
aivyx memory search <query> [--limit N]
aivyx memory evict <topic> [--yes]   # delete a whole topic
```

The Web UI Memory tab gives the same: a topic list, per-topic
entry view, an inline search bar, and a per-topic Evict
button (confirm-gated).

**Per-topic retention.** `[[memory.retention]]` blocks declare
a topic-glob pattern + a policy. The hourly GC walks every
entry, applies the **first** matching rule (put narrower globs
first), and falls through to the global `[memory] ttl_secs`
for unmatched topics:

```toml
[[memory.retention]]
topic_glob = "project/**"
retention = "forever"          # never TTL-expire

[[memory.retention]]
topic_glob = "notes/daily/*"
retention_days = 30            # evict entries older than 30d
```

Exactly one of `retention = "forever"` or `retention_days = N`
per block; partial / both-form / unknown-value config rejects
at load time.

**LRU eviction.** When a topic exceeds `[memory]
max_per_topic`, the least-recently-**read** entry is evicted
first. Every `memory.read` stamps `last_read_at`; the Web UI
Memory pane surfaces it ("last read: never" for entries the
agent has written but not recalled). No config knob — it's
automatic once `max_per_topic` is set. Distinct from the
prior FIFO-on-write eviction: an old note the agent keeps
recalling now survives a younger note it never reads.

### Topic canonicalization (Phase 89)

For 88 phases the assistant has accumulated topic-keyed
signal everywhere (Phase 7 memory, Phase 77 recall log,
Phase 82 helpfulness ledger, Phase 83 co-occurrence ledger,
Phase 87 consolidate-pair proposal IDs) — but every layer
keyed by the operator's typed topic string verbatim. That
means `deploy`, `Deploy`, `deploys`, and `deploying` are
four distinct topics across every accumulator, and the
signal that should add up across them was silently
fragmented.

Phase 89 closes the long-standing Phase 82 deferral with the
smallest possible substrate fix: an **opt-in canonicalization
seam at the `Memory` trait boundary**. With the flag on,
every topic-string argument is folded to a canonical form
before storage, and every topic-keyed lookup folds the same
way — so `Deploys`-the-write is found by `deploy`-the-read,
and the downstream signals (recall log, helpfulness ledger,
co-occurrence ledger, Persona facet provenance) all inherit
clean keys through the existing pipeline. No per-layer
plumbing; one seam, every consumer benefits.

- **Off by default.** With no `[memory] canonicalize_topics`
  key (or set to `false`), the memory layer is byte-identical
  to pre-Phase-89. Matches the project's 88-phase
  behaviour-change-is-opt-in discipline.
- **No migration.** Existing fragmented data stays as-is and
  decays out naturally via the Phase 82/83 ~60-day half-life
  + the Phase 77 ~30-day recall-log retention. The past
  converges to clean within roughly a quarter without
  intervention; no MAC-signed Persona chain entries are
  rewritten.
- **The v1 rule set.** A small hand-rolled English stemmer
  (no new workspace deps). Lowercase + trim + collapse
  whitespace, then **one** suffix-strip rule fires with
  min-length guards: `ies → y` (`policies → policy`),
  `ing` drop (`testing → test`), `ed` drop (`tested →
  test`), `es` drop **only when the stem ends in a
  hissing-sound letter — `sh` / `ch` / `s` / `x` / `z`**
  (`boxes → box`; `roles` falls through to the next rule),
  `s` drop (`tests → test`; `process` stays — the `ss`
  guard skips). The function is idempotent.
- **Applies to every topic-string trait entry point.**
  `put`, `get_recent`, `forget`, `gc_topic`,
  `evict_oldest_unread`, `put_vector`,
  `promote_recall_helpful`. **Does not** apply to prefix
  matching (`scan_prefix`), text queries (`search`), or
  topic-less methods (`gc_expired`, `list_topics`, the
  vector-only `semantic_search` paths).

Enable it in `~/.config/aivyx/aivyx.toml`:

```toml
[memory]
canonicalize_topics = true   # optional, default false
```

**To keep pre-Phase-89 behaviour:** leave the key out (or
set `false`). **Trade-off:** the stemmer lowercases proper-
noun-looking topics too (`Deploy` → `deploy`). The codebase's
typical topic slugs are category labels (`auth`, `frontend`,
`tests`), not entity names — the assumption is positive in
practice; an operator who needs case-sensitive topics
declines the opt-in. An operator-tunable alias table
(`[[topic_alias]]`) and a topic-by-topic exception list are
likely follow-ups if real-world fragmentation cases need them.

## Semantic memory search (Phase 75)

Phase 75 adds **embedding-ranked** memory retrieval on top of
the Phase 74 keyword search. It is **off by default** — without
an `[embedding]` section nothing changes, and a `semantic`
request transparently falls back to keyword.

**The base_url privacy choice.** Embedding means sending the
text to be embedded to the configured endpoint. `base_url` is
the only thing that decides whether memory content leaves the
machine:

- **Cloud** (`base_url = "https://api.openai.com"`, the
  default) — entry bodies and search queries are sent to
  OpenAI. This is your explicit, opt-in choice by configuring
  the section.
- **Local / on-device** — point `base_url` at any
  OpenAI-compatible server (ollama, llama.cpp,
  text-embeddings-inference, e.g.
  `http://localhost:11434`). Nothing leaves the box; no
  `api_key` needed.

```toml
[embedding]
base_url = "http://localhost:11434"   # local → on-device
model = "nomic-embed-text"
dimensions = 768
# api_key resolves env (AIVYX_EMBEDDING_API_KEY) > this TOML
# key > the encrypted secrets store, same as the LLM keys.
```

`dimensions` must match the model's native output. Omitted
fields default to `https://api.openai.com` /
`text-embedding-3-small` / `1536`.

**Keyword fallback.** A `semantic` request silently serves
keyword results — flagged so you can see it — when (a) no
`[embedding]` section is configured, (b) the embedding
provider call fails (down, rate-limited, bad key), or (c) the
vector index is still empty. You never get a hard error for
asking for semantic; you get the best available answer.

```
aivyx memory search "<query>" --semantic [--limit N]
```

The agent's `memory.search` tool gains a `mode` argument
(`"keyword"` default, `"semantic"`); the Web UI Memory tab
gains a **semantic** toggle next to the search box. All three
share one provider and one fallback rule.

**Backfill on upgrade.** Enabling `[embedding]` on an existing
store does not require a re-index command. New writes are
embedded inline; an hourly backfill pass (bounded per tick, so
no startup stall) walks entries lacking a current-dimension
vector and embeds them, so the back-catalog becomes searchable
gradually. Swapping embedding models is safe — vectors of the
old dimension are detected as stale and re-embedded by the
same backfill.

### ANN index for semantic memory search (Phase 96)

By default semantic memory search is **brute-force cosine**:
every query compares against every stored vector. That's
O(N) per query and works for any operator with up to a few
thousand entries. Phase 96 adds opt-in **IVF-style
approximate-nearest-neighbor** indexing for larger stores.

How it works: at build time, the vector index is partitioned
into K ≈ √N clusters via deterministic spaced-sampling +
one-pass nearest-centroid assignment. At query time, the
query vector is cosine-ranked against the K centroids
(cheap — K is small), the top-N clusters are selected, and
brute-force cosine then runs only within those clusters'
members. The candidate set narrows to ≈ N · (top_N / K),
which the existing brute-force re-rank then orders
**exactly** within that pool. End-to-end: O(N) → O(√N).

```toml
[embedding]
# ... existing fields ...
ann_index = true                # optional, default false
ann_rebuild_threshold = 100     # optional, default 100
```

**Semantics:**
- `ann_index = false` (default): brute-force only,
  byte-identical to pre-Phase-96.
- `ann_index = true`: ANN narrows candidates → brute-force
  re-ranks within. The final top-K is **exactly** ordered
  within the candidate set (the hybrid composition
  preserves the exact-cosine guarantee).

**Stale-rebuild:** the index lives in-memory and is rebuilt
on demand. `ann_rebuild_threshold = 100` means "after 100
new vector writes, the next recall rebuilds the index."
Lower the threshold for tighter freshness; raise it for
fewer rebuilds. Daemon restart drops the in-memory index;
the first ANN query after restart rebuilds from the
persisted embeddings.

**Quality:** IVF with one-pass nearest-centroid assignment
trades some recall for simplicity + zero new dependencies.
For very large stores (>100K entries) where recall quality
matters more, HNSW-level indexing is documented as a
Phase 96 deferral.

**To turn it off:** delete the `ann_index` key or set it to
`false`. `semantic_search_scored` returns to brute-force
byte-identically.

## Automatic recall (Phase 76)

Phase 75 gave the agent a semantic-search *tool*. Phase 76
makes recall **automatic**: with `[embedding]` configured,
every turn the assistant embeds your message, finds the most
semantically-relevant past memories, and injects them into
that turn's context **without being asked**. This is what
gives it continuity — it remembers across turns the way a
personal assistant should.

**Off when embedding is off.** No `[embedding]` section → no
auto-recall → behavior is byte-identical to pre-Phase-76. The
hook is also fully best-effort: an embedding-provider hiccup,
an empty vector index, or no sufficiently-relevant memory all
leave the turn untouched. Auto-recall never errors a turn.

**Two knobs** (under `[embedding]`, both optional):

- `rag_top_k` (default 5) — the most memories injected per
  turn.
- `rag_min_similarity` (default 0.20) — the cosine-similarity
  floor. This is the important one: it drops weakly-related
  hits so an unrelated prompt doesn't drag in noise. Raise it
  for stricter recall, lower it to recall more aggressively.
  Range `[0.0, 1.0]`; `rag_top_k` must be ≥ 1.

**What you'll see.** The recalled memories appear as a clearly
labeled, reference-only block at the top of the turn (visible
in the conversation / Web UI as part of that turn), and the
daemon log prints a one-line marker when recall fires:

```
aivyx recall: injected 3 memories [project/notes, prefs]
```

The block is explicitly framed to the model as background
reference, not instructions — a recalled note cannot hijack
the turn.

### Heuristic recall gate (Phase 90)

For 89 phases auto-recall and adaptive Persona selection
(Phase 79) fired on **every** conversational turn —
including turns where the user message is a one- or two-token
acknowledgment (`ok` / `thanks` / `yes` / `cool`) that
cannot meaningfully steer recall. The bare-message embed on
those turns is essentially a random vector that pollutes the
ranker; the recall block and adaptive Persona selection
injected on top are noise the planner has to defend against.

Phase 90 adds the smallest possible fix: a length-based
**heuristic gate** at the top of both relevance hooks that
skips the embed (and everything downstream) when the trimmed
user message is shorter than `recall_gate_min_chars`. Both
consumers (auto-recall + adaptive Persona) share the same
gate and the same opt-in knob, exactly as Phase 86's window
work shipped both consumers under one switch.

- **Opt-in, off by default.** With `recall_gate_min_chars =
  0` (the default) the gate is disabled and behaviour is
  byte-identical to pre-Phase-90. Raise it (`4` is a
  conservative starting point that gates single-token
  acknowledgments without affecting normal messages) to
  engage.
- **Same gate, both consumers.** A gated turn produces no
  recall block AND no adaptive Persona facet selection
  (the planner uses the full Persona base prompt, exactly
  the pre-Phase-79 fallback). Symmetric Phase 86 design.
- **Maximum cost saving.** A gated turn skips the embed
  call entirely (not just the memory walk or the ranking)
  — the cheapest possible noise-turn path.
- **Unicode-char counted.** The threshold is in characters,
  not bytes — `héllo` is 5 characters whether you measure
  it semantically or not.

Configure under `[embedding]`:

```toml
[embedding]
# ... existing knobs ...
recall_gate_min_chars = 4   # optional, default 0 (disabled)
```

**Trade-off.** A short but meaningful message (`run!`,
`ack`, `git`) gets gated alongside fillers. The current
heuristic is operator-tunable but not pattern-aware; an
operator who needs more nuance can keep the gate at `0` or
configure a low threshold (`2` or `3`) that catches only
the very shortest noise turns.

### Token-budget context sizing (Phase 97)

Auto-recall and adaptive Persona selection have always
capped injection by **entry count** (`rag_top_k` for
recall, an internal K-facet limit for adaptive Persona).
Count is a proxy for token cost, not the cost itself. A
single memory body with a 4 KB blob silently displaces
multiple shorter memories from the same `rag_top_k`
budget; a Persona facet that grew from one sentence to ten
paragraphs eats turn after turn of input — sometimes
enough to bump the prompt past the model's context limit.

Phase 97 adds an opt-in **token budget** that caps both
paths after their existing rank-and-filter steps. The
existing count caps remain in place as **soft hints**;
the token budget is the hard cap. Items are already in
rank order (cosine score for recall, selection priority
for Persona); the budget walks them, and the **first
item whose addition would exceed the budget** (along with
every item after it) is dropped. No mid-item truncation
— operators get full items or nothing.

```toml
[embedding]
# ... existing knobs ...
recall_token_budget = 2000   # optional, default 0 (disabled)
```

**Semantics:**
- `recall_token_budget = 0` (the default): no budget
  enforcement; behaviour is byte-identical to
  pre-Phase-97.
- `recall_token_budget = N` (any `N >= 1`): both recall
  and Persona injection drop their lowest-ranked items
  until the running estimate fits.

**Estimator:** hand-rolled `chars / 4` (the OpenAI rule-
of-thumb for English) with a small fudge factor.
Accuracy ~±20%; sub-token accuracy isn't worth a new
tokenizer dependency. Unicode `chars()`-counted, not
bytes.

**What the operator sees:** the existing recall
breadcrumb (`aivyx recall: injected N memor[y|ies]`)
reflects the post-budget set, so observers match what
was actually injected. The Phase 78 learning surface +
the Phase 84 cluster stat + the Phase 77 recall_log all
see the same post-budget hits.

**Edge case:** if every hit falls out of the budget,
auto-recall returns no block (the planner falls back to
the base prompt without an empty recall section).
Adaptive Persona's protected core (constraints + scalar
identity) is **always** present regardless of the
budget — the budget only trims soft-facet selection.

### Hybrid keyword+semantic recall (Phase 98)

Auto-recall has ranked by cosine similarity over
embeddings since Phase 75. Embeddings encode semantic
relationships well but struggle with **rare-term recall**:
acronyms, proper nouns, code identifiers, project
codenames. A query mentioning "ATC-417" or "kubernetes"
or "Jane Henderson" may miss the memory specifically
about that term because the embedding doesn't strongly
link the rare token to a learnable concept.

The keyword search tool (Phase 74,
`Memory::search`) handles these exact-match cases via
case-insensitive substring matching, but operates as a
**separate manual path** — the agent / operator drives
`aivyx memory search`, not auto-recall.

Phase 98 closes that gap with **Reciprocal Rank Fusion
(RRF)**. With `[embedding].recall_hybrid = true`,
auto-recall runs both the semantic ranker AND the
substring search on every recall, then fuses the two
rankings before feeding the downstream pipeline
(cluster expansion, token budget, etc.).

```toml
[embedding]
# ... existing knobs ...
recall_hybrid = true   # optional, default false
```

**Why RRF over score fusion.** Cosine scores in
`[-1, 1]` and substring hit counts don't share a scale.
Score-fusion approaches (`α * cosine + (1-α) *
keyword_score`) require normalization and an alpha tuning
knob. RRF is **rank-based** — it sums each item's
position-based contribution
(`1 / (k + rank + 1)` with `k = 60`, the industry-
standard constant) and ignores raw scores entirely. No
normalization, no tuning, no new dependency.

**Semantics:**
- `recall_hybrid = false` (default): semantic-only,
  byte-identical to pre-Phase-98.
- `recall_hybrid = true`: both rankers run; their
  rankings fuse via RRF; the fused top-K feeds the
  downstream pipeline.

**The `rag_min_similarity` floor.** RRF scores aren't on
the cosine scale, so the configured similarity floor
isn't directly comparable. v1 **skips** the floor on the
hybrid path. The `rag_top_k` cap still limits the fused
output, and items that only one ranker surfaces tend to
get small RRF scores (`1/61 ≈ 0.0164`) that get pushed
out by stronger items. A future phase could add a
separate `rag_hybrid_min_rrf` knob.

**What this fixes:** queries with rare or technical
terms now reliably surface memories about those terms,
even when the semantic side doesn't rate them highly.
The semantic side still catches the conceptually
similar memories. The fusion is the union of both
signals.

## Recall feedback loop (Phase 77)

Auto-recall (Phase 76) made the assistant *remember*. Phase 77
makes it **learn which memories are worth remembering** —
without you configuring anything and without an LLM grading
itself.

**How it learns (structurally, no LLM).** Every auto-recall is
logged. On your existing reflection schedule's cron, the loop
correlates each recall with how that turn actually went, using
only signals already in the audit chain:

- the turn `completed` and you did **not** immediately come
  back → the recalled memories scored **helpful**;
- the turn `failed`/`timed_out`, **or** you started another
  turn in the same session within 60 s (the structural proxy
  for "that didn't land") → scored **unhelpful**;
- `escalated`/`cancelled` → no signal.

It never asks the model whether its own recall was useful —
that self-judgement is exactly what this avoids. Per-turn the
signal is coarse; across many turns it is reliable.

**What it does with the signal — two actuators:**

1. **Memory retention self-tunes.** Consistently-helpful
   memories are kept "warm" so the existing Phase 74 LRU
   eviction protects them; consistently-unhelpful ones are
   simply not protected and age out under the same pass. No
   new eviction policy — good memory just gets stickier.
2. **Operator-gated Persona proposals.** A topic whose
   memories are *strongly* and repeatedly helpful files a
   **Pending** Persona proposal (e.g. "operator consistently
   benefits from recalled context about X — keep surfacing
   it"). You review and approve or reject it via the existing
   `aivyx persona proposals` flow. **The loop never edits the
   Persona itself** — you remain the authority (the Phase 70
   P14 rule). The same deterministic proposal is filed once;
   it won't re-nag after a rejection.

**Zero configuration.** There is no `[recall_feedback]`
block — thresholds and the ~30-day recall-event retention are
fixed for v1. The loop is active precisely when auto-recall
(`[embedding]`) **and** a `[[reflection_schedule]]` are both
present; otherwise it is a complete no-op (pre-Phase-77
behavior). Each cycle prints a daemon-log breadcrumb:

```
aivyx recall-feedback: schedule "nightly" — 12 entries scored, 4 promoted, 1 proposal(s) filed
```

### LLM-judged recall usefulness (Phase 91)

For 90 phases the recall-feedback signal above has been
**structural**: every recall in a successfully-completed turn
inherits `+1` helpfulness; every recall in a failed turn
inherits `-1`. Phase 91 adds an opt-in **LLM-judged per-recall
classification** alongside the structural proxy — a 3-way
verdict (`used` / `irrelevant` / `hurt`) recorded on each
recall hit so downstream consumers can eventually consult a
sharper signal than turn-level outcome.

After the input-quality arc (86 windows, 89 canonicalization,
90 recall gate), Phase 91 is the missing-half **feedback-
quality** move:

- **Phase 86** sharpened *what* gets embedded (windows).
- **Phase 89** sharpened *how* signals key (canonical topics).
- **Phase 90** sharpened *when* recall fires at all.
- **Phase 91** sharpens *whether* recall actually helped.

Important: Phase 91 is **augment, not replace** (Q3a). The
new `judgment: Option<RecallJudgment>` field is captured on
every recall hit, but no existing accumulator (Phase 82
helpfulness ledger, Phase 83 co-occurrence ledger, Phase 85/88
Persona decay, Phase 87 pattern-driven proposals) reads it in
v1. Every existing behaviour stays byte-identical. A future
phase consumes the new signal once it is validated in
production.

- **Off by default.** With no `[recall_judgment]` block (or
  `enabled = false`) the LLM judge never runs — zero added
  cost, zero behaviour change. Matches the 90-phase
  behaviour-change-is-opt-in discipline.
- **Reflection-cron batched.** One LLM call per cron tick
  judges every unjudged recall in the lookback window (up to
  `max_recalls_per_cycle`, default `30`); the remainder rolls
  to the next cycle. Bounded cost shape, identical to Phase
  87's `LlmPairPhraser` cadence.
- **v1 simplification.** The judge classifies based on the
  recall's `(topic, body)` content + a weak context hint —
  it does NOT see the model's actual response text (which
  isn't in the audit chain today). A future phase enriches
  the input via audit-chain extension or per-turn capture;
  the Q3a augment posture means even this weaker v1 judgment
  changes nothing it shouldn't.
- **Operator-visible.** Each cycle prints a breadcrumb
  (`aivyx recall-judgment: schedule "nightly" — judged 12
  (used=7, irrelevant=4, hurt=1, skipped=0)`); `aivyx
  learning` + the Web UI Learning tab render a new
  "LLM-judged recall usefulness (last cycle, opt-in)" block
  showing per-classification counts + the `(topic, judgment)`
  pairs.

Enable it in `~/.config/aivyx/aivyx.toml`:

```toml
[recall_judgment]
enabled = true
max_recalls_per_cycle = 30   # optional, default 30
```

Validation (only when `enabled = true`):
`max_recalls_per_cycle >= 1`. **To turn it off:** set
`enabled = false` or delete the block — every accumulator
returns to pre-Phase-91 behaviour.

### Judgment-driven recall feedback (Phase 93)

Phase 91 records per-hit `RecallJudgment` (`Used` /
`Irrelevant` / `Hurt`) on every recall hit. Phase 93 lets the
**recall-feedback actuator** consume those verdicts. With
the new `[recall_feedback].use_judgment_signal = true` knob,
the correlator (`correlate_detailed`) reads each hit's
`judgment` field and uses it to derive that hit's signal —
overriding the Phase 77 turn-level structural proxy for any
hit that carries one. Un-judged hits keep using the
structural proxy, so the augment is incremental as the
Phase 91 cron processes hits.

Per-verdict mapping (symmetric with the structural
`±WEIGHT`):

- `Used` → `+WEIGHT` (the hit was helpful, regardless of
  the turn-level outcome).
- `Hurt` → `-WEIGHT` (the hit was actively misleading,
  regardless of the turn-level outcome).
- `Irrelevant` → no contribution (dead weight; neither
  rewarded nor punished).
- `None` (un-judged) → falls back to the turn-level
  structural signal.

The downstream actuators (memory promotion via
`apply_retention_feedback`, Persona proposals via
`emit_persona_proposals`) read the same `HelpfulnessTally`
shape — only the signal source per hit changes. Operators
who enabled Phase 91 for *visibility only* (the v1
"augment, not replace" posture documented at field
introduction) see no actuator-behaviour change unless they
also flip this knob.

```toml
[recall_feedback]
use_judgment_signal = true   # optional, default false — Phase 93
```

The knob lives in `[recall_feedback]` (the consumer side),
separate from `[recall_judgment]` (the producer side from
Phase 91), so the two configs stay independently
reason-aboutable. Turning the judge on without flipping
this knob keeps the actuator on the structural signal it
has used since Phase 77; flipping both turns on the
self-improving loop end-to-end. **To turn it off:** set
`use_judgment_signal = false` or delete the block —
`correlate_detailed` returns to byte-identical pre-Phase-93
behaviour.

The `aivyx learning` surface flags the augment with a
`signal source: judgment-driven` banner under the recall
count when the knob is on, so the operator can confirm at
a glance that the loop is in the augmented mode they
expect.

## Tool observability (Phase 102)

`aivyx tools` is the read-only window onto the tool layer —
the sibling of `aivyx learning`:

```
aivyx tools [--window <secs>]
```

It lists every registered tool and annotates each with
audit-derived call statistics: total calls, the outcome
breakdown (completed / failed / denied / …), and average
call duration. `--window <secs>` scopes the stats to a
recent slice; without it the whole audit chain is summed. A
tool that has never been called still appears — a
registered-but-unused tool is itself a signal — and a row
marked `[unregistered]` is a capability base with call
history but no currently registered tool. Like `aivyx
memory` and `aivyx learning`, it is daemon-backed: it needs
a running daemon (`aivyx daemon run`).

## Learning insights (Phase 78)

The Phase 77 loop changes behaviour on its own. Phase 78 makes
that **legible** — an autonomous system you can't see is one
you can't trust. There's a read-only view of *what the
assistant has learned and why*, with full CLI + Web UI parity:

```
aivyx learning [--window <secs>]
```

and a **Learning** tab in the Web UI. Both show the same two
things:

- **A digest** — over the lookback window: how many recalls
  happened and how many scored, how many memories the
  retention actuator is keeping warm vs. letting age out, the
  count of recall-driven Persona proposals, and the top
  helpful / least-helpful topics. This answers "is the loop
  healthy and what is it leaning toward."
- **Proposal provenance** — for each Pending (or resolved)
  recall-driven Persona proposal: the topic, its net score,
  the agent's stated reason, and the actual recalls/turns that
  produced the score (timestamp, turn outcome, whether each
  helped or hurt). This answers "*why* did it propose to
  change its Persona" — the highest-trust-stakes question,
  since you approve/reject those proposals.

It is **read-only**: approve/reject still happens through
`aivyx persona proposals` / the Proposals pane. Nothing here
is configurable and nothing is persisted for it — the view is
computed on demand from the live recall log, so it always
matches what the loop actually did. The horizon is bounded by
the ~30-day recall-event retention; with no `[embedding]` /
no recall yet, it simply reports an empty digest (a valid
"nothing learned yet", not an error).

```
aivyx learning --window 604800   # last 7 days
```

## Adaptive Persona (Phase 79)

Before Phase 79 the **entire** accreted Persona — every learned
context note, character trait, communication adaptation the
reflection loop has ever written — was injected into *every*
system prompt, unbounded and identical regardless of the turn.
As the Soul matures over months that grows without limit and
dilutes its own signal. Phase 79 makes it **adaptive**: each
turn the assistant injects only the Persona facets
semantically relevant to your message.

**The always-on core (the safety invariant).** Selection only
ever applies to the *soft* list facets. The scalar identity
(`assistant_name`, `operator_profile`, `communication_style`)
and **every `behavioral_constraint`** are injected in full on
every turn, unconditionally — they can never be selected away.
Your declared identity and your guardrails always apply; only
which *learned* facets surface is contextual.

**Only engages when it matters.** With no `[embedding]`
configured, **or** while the Soul is still small (below an
internal facet threshold), the full Persona is injected
exactly as before — byte-identical to pre-Phase-79. The
feature is invisible until the Persona is actually large
enough to need bounding; an embed failure also falls back
silently. It is never a regression and never an error. There
is nothing to configure (a `[persona]` tuning block is a
deferred follow-up).

**Where to see it.** Each turn the daemon log prints
`aivyx persona: injected N/M facets`, and the same
selected/total appears in the `aivyx learning` view and the
Web UI **Learning** tab ("Adaptive Persona: N/M facets
injected last turn") — the Phase 78 trust surface, extended:
an adaptive Soul stays legible.

## Proactive surfacing (Phase 80)

For 79 phases the assistant only ever acted when prompted — a
turn, a cron, a webhook. Phase 80 lets it **reach out first**:
on its existing reflection cadence it notices a concrete,
high-confidence reason to surface something and sends it
unprompted — *"you noted X 29 days ago, it expires
tomorrow"*; *"your `deploy/` notes keep helping, here's the
cluster."*

An unprompted **outbound** message is the highest-trust-stakes
thing the assistant can do, so it ships **off by default,
hard-capped, and fully explainable**:

- **Opt-in.** With no `[proactive]` section (or
  `enabled = false`) the pass is a complete no-op — exactly
  pre-Phase-80 behaviour. Nothing reaches out unless you ask
  it to.
- **Structural gate, no extra LLM.** It surfaces only when it
  can point to a concrete reason in one of three conservative
  signal classes: a memory within a day of TTL eviction
  (`signal_ttl_expiry`), a topic whose Phase 77 net
  helpfulness is strongly positive (`signal_recall_cluster`),
  or a `@due:`-marked reminder whose time has arrived
  (`signal_due_reminder`). No model judges *whether* to
  interrupt you — the Phase 77 no-self-judgement ethos applied
  to the highest-stakes action.
- **Hard volume cap.** At most `max_per_window` sends per
  `window_secs` (default **3 per day**), enforced
  deterministically on top of Phase 73's per-target
  rate-limit. Proactive is a scalpel, not a feed.
- **Never nags.** Every surfaced item's deterministic id is
  recorded in an encrypted, HKDF-isolated `ProactiveLog`
  store; the same item is never surfaced twice across cycles.
  Rows GC on the reflection cadence (~30-day retain).

**Configure it** in `~/.config/aivyx/aivyx.toml`:

```toml
[proactive]
enabled = true
target  = "me"          # a configured notify target name
max_per_window = 3       # optional, default 3
window_secs    = 86400   # optional, default 86400 (1 day)
# Each signal class defaults ON when proactive is enabled;
# set to false to mute one. At least one must stay on.
signal_ttl_expiry     = true
signal_recall_cluster = true
signal_due_reminder   = true
```

Validation (only when `enabled = true`): `target` non-empty,
`max_per_window >= 1`, `window_secs >= 1`, at least one signal
class on. **To turn it off:** set `enabled = false` or delete
the `[proactive]` block.

**Where it's recorded.** Every send lands in the notify
history (the existing `AutoNotifyDispatched` audit event, same
as any auto-notify). The daemon log prints a per-cycle
breadcrumb `aivyx proactive: schedule … — surfaced N
(deduped D, capped C)`, and the last cycle's outcome — items,
their `reason` provenance, dedup/cap counts — appears in the
`aivyx learning` view and the Web UI **Learning** tab
("proactive: N surfaced last cycle"), the Phase 78 trust
surface extended once more.

## Persona lifecycle (Phase 81)

For 80 phases the Persona ("Soul") only ever **grew** — the
reflection loop adds facets, none ever consolidated a
redundant one or retired a stale one. Over months a Soul that
only accretes dilutes its own signal and can contradict
itself; Phase 80 raised the stakes (a bloated Soul now also
drives proactive sends). Phase 81 gives the Persona a
**lifecycle**: on the existing reflection cadence the
assistant notices near-duplicate and long-unreinforced
soft-list facets and **proposes** consolidation or decay.

Identity is the highest-stakes layer, so it ships **off by
default, propose-only, core-protected, and fully reversible**:

- **Opt-in.** With no `[persona_lifecycle]` section (or
  `enabled = false`) the pass is a complete no-op — exactly
  pre-Phase-81 behaviour. It also needs a
  `[[reflection_schedule]]` (it piggybacks that cron) and an
  `[embedding]` provider (consolidation embeds facets).
- **Propose-only — the loop never edits identity.** Every
  action is filed as a normal *Pending* `PersonaProposal` you
  approve or reject in `aivyx persona` / the Web UI. Nothing
  changes the Soul until you say so, and `aivyx persona
  revert` undoes any approved action (it is a plain
  `RemoveList` delta on the chain).
- **The always-on core is structurally untouchable.** Only
  the six *soft* lists (`primary_use_cases`,
  `behavioral_preferences`, `learned_context`,
  `communication_adaptations`, `character_traits`,
  `relationship_milestones`) are ever considered. The scalar
  identity and **every `behavioral_constraint`** are excluded
  by construction — the Phase 79 always-on-core invariant
  extended to this layer.
- **Conservative, no extra LLM.** *Consolidate*: facets whose
  embeddings are near-identical (cosine above
  `consolidation_similarity`) — it proposes removing the
  shorter near-duplicates and keeping the longest (canonical)
  one. *Decay*: a facet whose originating delta is older than
  `decay_max_age_secs` with no later delta in its category
  (active curation suppresses decay). Never acts on a list
  with fewer than `min_soft_facets` entries. No model judges
  *whether* to act.
- **Never nags.** A deterministic proposal id means an action
  already filed (in any status, including a prior *Rejected*)
  is never re-proposed.

**Configure it** in `~/.config/aivyx/aivyx.toml`:

```toml
[persona_lifecycle]
enabled = true
consolidation_similarity = 0.92   # optional, default 0.92
decay_max_age_secs = 7776000      # optional, default ~90d
min_soft_facets = 6               # optional, default 6
# Each class defaults ON when enabled; set false to mute one.
# At least one must stay on.
signal_consolidate = true
signal_decay = true
```

Validation (only when `enabled = true`):
`consolidation_similarity` in `(0.0, 1.0]`,
`decay_max_age_secs >= 1`, `min_soft_facets >= 1`, at least
one signal class on. **To turn it off:** set
`enabled = false` or delete the `[persona_lifecycle]` block.

**Where to see it.** The daemon log prints
`aivyx persona-lifecycle: schedule … — proposed N
(deduped D)`, and the last cycle's proposed actions + their
`reason` provenance appear in the `aivyx learning` view and
the Web UI **Learning** tab ("persona lifecycle: N proposed
last cycle"). Filed proposals show up in `aivyx persona`
exactly like reflection-driven ones — the Phase 78 trust
surface extended once more.

### Helpfulness-driven decay (Phase 85)

Phase 81 decay is **age-only** — a weak proxy. Phase 85 makes
decay consult the durable Phase 82 helpfulness ledger so the
Soul retires identity that **demonstrably stopped helping**,
and *keeps* old identity that **still helps**:

- **Precise, not fuzzy.** Only a facet whose recall topic is
  *known exactly* is helpfulness-gated. Facets the
  recall-feedback loop produced carry a `recall-fb:{topic}`
  provenance that survives onto the persona chain; everything
  else (reflection-authored facets — no topic linkage) stays
  age-only, byte-identical to Phase 81.
- **Symmetric.** A topic with sustained-negative helpfulness
  can trigger its facet's decay *before* the age horizon (a
  facet that keeps hurting shouldn't wait a quarter);
  symmetrically, a sustained-*positive* topic **protects** an
  age-old facet from age-decay.
- **Conservative evidence.** "Sustained" means the topic's
  decayed ledger score is at/below `decay_unhelpful_threshold`
  (negative) **and** it has at least `decay_min_samples`
  observations — identity is never retired (or protected) on
  thin evidence.
- **Same safety posture.** Still propose-only, operator-gated,
  `Revert`-able, core-protected — every Phase 81 property is
  unchanged. With no helpfulness ledger it degrades gracefully
  to pure age-only.

Two optional knobs on the **same `[persona_lifecycle]`**
block (gated by the existing `signal_decay`):

```toml
[persona_lifecycle]
enabled = true
# … Phase 81 knobs …
decay_unhelpful_threshold = -2.0  # optional, default -2.0
decay_min_samples = 3             # optional, default 3
```

Validation (only when enabled and `signal_decay` is on):
`decay_unhelpful_threshold < 0.0`, `decay_min_samples >= 1`.
**To keep pure age-only behaviour:** don't run auto-recall
(no ledger), or leave the knobs at defaults — a facet is only
ever helpfulness-decayed when its `recall-fb` topic has
genuinely, sustainedly hurt. Decay proposals cite the
evidence (e.g. *"topic 'deploy' net -8.2 over 14 windows
(sustained low helpfulness)"*) in the same `aivyx persona` /
Phase 78 surface.

### Pattern-driven decay (Phase 88)

The decay-side complement of Phase 87's pattern-driven
proposals. Phase 87 makes the co-occurrence ledger drive
Persona *construction* — a durable affined pair proposes a
new `learned_context` facet; Phase 88 makes the **same
ledger** drive Persona *decay*: when the pair underlying an
already-applied `consolidate-pair:` facet has demonstrably
weakened, the facet's justification is gone — propose to
retire it. The opposite move lands symmetrically: a still-
durable pair **protects** its facet from age-decay (the
relationship still applies, so the identity still applies).

After Phase 88, the assistant retires identity when the
**relationship** behind it dissolves — not only when the
underlying *topic* stopped helping. Every existing safety
property carries: propose-only, operator-gated, `Revert`-
able, core-protected.

- **Conservative, single-signal gate.** The pair's decayed
  Phase 83 affinity must be **below** the
  `decay_pair_below_affinity` floor (default `1.0` — mirrors
  Phase 87's `min_affinity` so the construction floor and the
  decay floor coincide by default). Endpoint helpfulness is
  **not** double-consulted: the facet's justification IS the
  relationship's durability, and tying decay to individual
  topic helpfulness would leave drifted-but-warm pair facets
  in place forever — the very case Phase 88 is meant to
  handle.
- **Symmetric protection.** A pair whose decayed affinity is
  *still* at or above the floor protects its `consolidate-
  pair:` facet from age-decay. Mirrors the Phase 85
  protection arm; reuses the same OR-protection machinery in
  the detector.
- **Provenance-only.** Only facets whose origin delta has a
  `consolidate-pair:{A}+{B}` proposal_id are pair-gated.
  Every other facet (reflection-authored, recall-feedback-
  derived) follows whatever signal it already had — age-only,
  or the Phase 85 helpfulness path.
- **Graceful fallback.** With no co-occurrence ledger (no
  auto-recall configured), the pair arm sits out entirely —
  byte-identical to Phase 85.

One new knob on the **same `[persona_lifecycle]`** block
(gated by the existing `signal_decay`):

```toml
[persona_lifecycle]
enabled = true
# … Phase 81 + 85 knobs …
decay_pair_below_affinity = 1.0  # optional, default 1.0
```

Validation (only when enabled and `signal_decay` is on):
`decay_pair_below_affinity` finite and ≥ `0.0`. **To widen
the keep-zone:** tune it *below* Phase 87's `min_affinity`
to add explicit hysteresis (e.g. propose at affinity ≥ 1.0,
decay only at affinity < 0.5). Decay proposals cite the
pair + the decayed affinity ("co-occurrence pair `deploy` +
`rollback` decayed affinity 0.30 (below floor 1.00);
relationship no longer durable") in the same `aivyx persona`
/ Phase 78 surface.

## Persistent helpfulness ledger (Phase 82)

For 81 phases the "did recalling this topic actually help"
signal was **ephemeral**: the recall-feedback loop (Phase 77)
recomputed it each reflection cycle over a lookback window and
discarded it. Phase 82 makes it **durable and longitudinal** —
a per-topic, time-decayed accumulation that survives restarts
and spans sessions, so the assistant can show you what has
*consistently* helped, not just what helped this week.

- **Zero-config and automatic.** Like the recall-feedback
  loop itself (Phase 77) and the recall log, there is **no
  `[helpfulness_ledger]` block** — nothing to turn on. It is
  built and folded automatically whenever auto-recall is
  configured (an `[embedding]` provider + a
  `[[reflection_schedule]]`). With auto-recall off it simply
  does not exist.
- **It changes no behaviour on its own.** It is a *passive*
  longitudinal signal. The recall-feedback loop's retention
  bias and Persona proposals are byte-identical to
  pre-Phase-82 — the ledger is folded in *after* those
  actuators run.
- **Recency-weighted (it forgets, on purpose).** Each
  reflection cycle the stored per-topic score is first decayed
  by an exponential half-life (~60 days), then this window's
  net helpfulness is added. A topic that used to help but
  hasn't lately fades on its own — the durable signal tracks
  *current* relevance, not a frozen all-time tally.
- **Self-pruning.** A topic whose decayed score has fallen to
  effectively zero *and* has not been touched for ~90 days is
  dropped on the same reflection cadence. Storage growth
  mirrors the signal's own decay; nothing accumulates forever.
- The half-life and prune bounds are code constants (tuning
  is a deferred follow-up, exactly as the recall-log's 30-day
  retention is fixed).

**Where to see it.** Run `aivyx learning [--window <secs>]` or
open the Web UI **Learning** tab: alongside the existing
*windowed* "Most/Least helpful topics" there is now an
**"Accumulated helpfulness (all-time, decayed)"** block — each
topic with its signed decayed score and a sample count (your
confidence proxy: one cycle is not a trend). The daemon log
prints `aivyx helpfulness-ledger: folded N topic(s), pruned M`
each cycle. Nothing to configure.

## Cross-session pattern learning (Phase 83)

Phase 77 learns *which topics help*; Phase 82 made that
durable. Phase 83 learns the relationships *between* topics:
which two topics get **recalled together** in turns that go
well. Over many sessions a stable picture emerges — "whenever
`deploy runbook` is recalled, `rollback steps` is too, and
those turns succeed" — and that is exactly the cross-session
structure a personal assistant should internalize.

- **Zero-config and automatic.** Like the recall-feedback
  loop (Phase 77) and the helpfulness ledger (Phase 82),
  there is **no config block** — it is built and folded
  automatically whenever auto-recall is configured (an
  `[embedding]` provider + a `[[reflection_schedule]]`). With
  auto-recall off it does not exist.
- **It changes no behaviour on its own.** A *passive*
  cross-session signal: it is folded in *after* the
  recall-feedback actuators and the Phase 82 ledger, so both
  remain byte-identical. (Acting on the patterns —
  cluster-aware recall, pattern-driven proposals — is a
  deliberate future phase.)
- **What a "pattern" is.** For each recall turn, the
  **top-8 highest-scoring distinct topics** are paired up;
  every unordered pair gets that turn's helpfulness signal
  (+ if it went well, − if not). The per-pair signal is
  accumulated with the same ~60-day exponential half-life as
  the Phase 82 ledger — a relationship that *used* to hold but
  hasn't lately fades on its own.
- **Bounded.** The top-8 cap keeps a turn that recalled 30
  memories from exploding into hundreds of pair rows; the
  ledger self-prunes (decayed-to-zero **and** ~90 days
  untouched → dropped), so storage tracks the live signal.

**Where to see it.** `aivyx learning` and the Web UI
**Learning** tab now show a **"Topics that consistently help
together"** block — each pair with its signed decayed score
and a sample count. The daemon log prints
`aivyx cooccurrence: folded N pair(s), pruned M` each cycle.
Nothing to configure.

## Cluster-aware co-recall (Phase 84)

Phases 82–83 built durable learning *surface-only*. Phase 84
is the first phase that **acts** on it. Auto-recall (Phase
76) surfaces only the memories whose text your turn
semantically matched. With cluster-aware co-recall, when a
topic A is recalled, the durable siblings B that have
*consistently helped alongside A across sessions* (the Phase
83 co-occurrence ledger) are **also** surfaced — even when
the literal query never retrieved them. Recall becomes
associative: the assistant brings what *goes with* what you
asked about, not just the keyword match.

Because this is the first time the assistant changes what the
model sees on the hot path, it ships **opt-in, hard-bounded,
budget-neutral, and self-policing**:

- **Opt-in.** With no `[recall_cluster]` section (or
  `enabled = false`) recall is byte-identical to pre-Phase-84.
  It also needs auto-recall configured (`[embedding]` +
  the co-occurrence ledger, which exists once auto-recall
  has been running).
- **Budget-neutral.** Injected siblings **share** the
  existing `rag_top_k` budget — they displace the *weakest*
  primary hits, so recall context never grows: zero extra
  token cost, no context bloat.
- **Hard-bounded.** At most `max_siblings` per turn, and only
  pairs whose decayed co-occurrence score clears
  `min_affinity` — weak/noisy affinities never reach context.
- **Self-policing.** Cluster-injected hits are marked and
  **excluded from the Phase 83 co-occurrence fold**, so the
  ledger never learns from its own expansion (no runaway
  self-reinforcement). They *do* count in the Phase 77/82
  helpfulness signal, so a bad expansion organically lands in
  worse turns and the affinity that drove it decays away.

**Configure it** in `~/.config/aivyx/aivyx.toml`:

```toml
[recall_cluster]
enabled = true
max_siblings = 3   # optional, default 3 — hard per-turn cap
min_affinity = 1.0 # optional, default 1.0 — decayed-score floor
```

Validation (only when `enabled = true`): `max_siblings >= 1`,
`min_affinity > 0.0`. **To turn it off:** set
`enabled = false` or delete the `[recall_cluster]` block.

**Where to see it.** The daemon log prints
`aivyx recall-cluster: injected N affined sibling(s)` on turns
that expand, and `aivyx learning` / the Web UI **Learning**
tab show a **"Cluster co-recall (last turn)"** block — the
injected count and each `driver → sibling` pair.

## Conversational-window relevance (Phase 86)

For 85 phases auto-recall (Phase 76) and adaptive Persona
selection (Phase 79) judged relevance off **one line** — the
latest user message. In a real multi-turn conversation the
topic drifts, the operator's intent spans several turns, and a
single line is a lossy proxy. Phase 86 gives both consumers a
**recent conversational window**: a small recency-ordered slice
of the last few turns (user + assistant) concatenated into the
*same* single embedding the relevance ranking already makes —
so recall pulls memories the multi-turn intent points at, and
the Soul selects facets matched to the actual thread of
conversation, not the literal last sentence.

The window is sharper *input* for the existing rankers; every
downstream guarantee (the `rag_min_similarity` floor, the
Phase 79 always-on-core invariant, the Phase 84 budget-neutral
sibling injection) is unchanged.

- **Opt-in, byte-identical by default.** A new
  `[embedding].recall_window_turns` knob defaults to `1` —
  exactly today's single-message behaviour. The window engages
  *only* when an operator raises it; no existing operator's
  recalled context changes on upgrade.
- **Recency, current message last.** When engaged, the
  embedded query is the last `recall_window_turns - 1` prior
  turns (oldest → newest, role-labelled `user:` / `assistant:`)
  followed by the current message — placed **last** so it
  dominates the embedding. Char-budgeted: prior turns are
  dropped oldest-first to fit; the current message is never
  truncated.
- **Ephemeral.** The buffer lives in daemon memory only —
  a restart starts fresh. Durable per-session transcripts are
  intentionally not persisted (recall context is re-derivable
  from memory + the Phase 82/83 ledgers; the *chatter* is not
  itself the record).
- **Applies to both consumers.** Auto-recall (Phase 76) and
  adaptive Persona selection (Phase 79) share the same handle
  and the same knob — the deferral came from both phases and
  fixing one without the other was incoherent.
- **Safety net unchanged.** A drifted window that drags in
  noise is filtered by the existing `rag_min_similarity` /
  Persona-selection floors; a stale window never injects a
  weakly-related memory or facet.

**Configure it** in `~/.config/aivyx/aivyx.toml`:

```toml
[embedding]
# … existing knobs …
recall_window_turns = 3   # optional, default 1 (= pre-Phase-86)
```

Validation (only when `[embedding]` is present):
`recall_window_turns >= 1`. **To turn it off:** leave the knob
at the default (or set `recall_window_turns = 1`) — recall and
Persona selection embed just the latest message, byte-identical
to the pre-Phase-86 path. The buffer caps the window at 16
turns regardless of the knob (the relevant signal is recency,
not a transcript).

## Pattern-driven Persona proposals (Phase 87)

Phase 84 made auto-recall **act** on the Phase 83 co-occurrence
ledger (durable affined siblings on the hot path). Phase 85
made Persona decay **act** on the Phase 82 helpfulness ledger
(sustained-negative topics retire identity). Phase 87 closes
the symmetric arc: the same co-occurrence ledger now drives
Persona *construction* too — durable, consistently-co-occurring
pairs of *helpful* topics propose a new `learned_context`
facet so the Soul learns the *relationships* between topics,
not just the per-topic warmth.

The actuator ships through the **existing Phase 70 proposal
chain** — same propose-only + edit-then-approve + `Revert` +
core-protected flow. Phase 87 only adds a new *source* of
proposals; the resolution path is unchanged.

- **Opt-in, off by default.** Like Phase 80/81/84, this is an
  actuator block. With no `[persona_consolidation]` section
  (or `enabled = false`) the pass never runs — byte-identical
  to pre-Phase-87. It also needs auto-recall configured (the
  Phase 82 helpfulness ledger and the Phase 83 co-occurrence
  ledger only exist once auto-recall has been running).
- **Conservative double-gate.** A pair `(A, B)` only proposes
  when its decayed Phase 83 affinity clears `min_affinity`
  AND has at least `min_samples` observations AND **both
  endpoints'** Phase 82 helpfulness scores are at least
  `min_topic_helpfulness`. A pattern of topics that
  individually hurt is never proposed (mirroring Phase 85's
  evidence-floor discipline).
- **LLM-summarized facets.** Each surviving pair is handed to
  the same reflection LLM the agent uses; it phrases one
  short factual statement (under 30 words) that lands as a
  Pending `LearnedContext` facet. The operator reviews — and
  may edit — the prose before approving. A per-candidate LLM
  hiccup skips that pair; a cycle-wide LLM outage is recorded
  on the **Learning** surface so you can distinguish a quiet
  cycle from a broken one.
- **Reflection cadence, dedupless, capped.** Runs on your
  existing `[[reflection_schedule]]` cron — the same trigger
  every "act on durable learning" pass uses (77, 82, 83, 85).
  Cross-cycle dedup is absolute: a pair already present in
  the proposal chain (any status — Pending, Approved,
  Rejected, Superseded) is never re-filed. Per-cycle filings
  are bounded by `max_proposals_per_cycle` (default `3`) so
  the review queue can never flood.

**Configure it** in `~/.config/aivyx/aivyx.toml`:

```toml
[persona_consolidation]
enabled = true
min_affinity = 1.0           # optional, default 1.0
min_samples = 3              # optional, default 3
min_topic_helpfulness = 0.0  # optional, default 0.0 (non-negative)
max_proposals_per_cycle = 3  # optional, default 3
```

Validation (only when `enabled = true`):
`min_affinity > 0.0`, `min_samples >= 1`,
`min_topic_helpfulness` finite,
`max_proposals_per_cycle >= 1`. **To turn it off:** set
`enabled = false` or delete the `[persona_consolidation]`
block — the Persona proposal pipeline is byte-identical to
pre-Phase-87.

**Where to see it.** The daemon log prints
`aivyx persona-consolidation: schedule "X" — filed N` on
cycles that fire (with `(LLM unavailable)` appended when the
LLM is unreachable). `aivyx learning` / the Web UI
**Learning** tab show a **"Pattern-driven Persona proposals
(last cycle, opt-in)"** block with the filed pair list, or
the engaged-but-quiet / LLM-down / off cases. The proposals
themselves appear in `aivyx persona proposals` + the
**Proposals** pane exactly like reflection-driven and
lifecycle proposals — each one with provenance citing the
specific co-occurrence pair (`co-occurrence pair X + Y —
decayed affinity N over M observation(s); both topics
helpful`).

### Pattern-driven supersession (Phase 92)

Phase 87 proposes new `consolidate-pair:` facets when a
durable + helpful pair shows up; Phase 88 decays old facets
when their pair weakens. But a real operator workflow shifts
continuously — `(auth, jwt)` dominates one quarter, then
`(auth, sessions)` the next. Today the actuator handles this
as **two independent operator decisions**: Phase 88 proposes
decay of the old facet, Phase 87 proposes the new one.
Nothing tells the operator they're logically linked.

Phase 92 adds opt-in **pattern-driven supersession**: when
an existing applied `consolidate-pair:{A}+{B}` facet's pair
has decayed below the Phase 88 floor AND a new pair
`(A, C)` sharing one endpoint has strengthened above the
Phase 87 floor (both endpoints helpful), the consolidation
pass files the `RemoveList` + `AppendList` proposals
**linked by metadata** so the operator-facing surface
presents them as a single supersession decision.

- **Opt-in, off by default.** With
  `[persona_consolidation].enable_supersession = false`
  (the default) the Phase 87 / Phase 88 proposal flow is
  byte-identical to pre-Phase-92.
- **Shared-endpoint detection.** The old pair `(A, B)` and
  the new pair `(A, C)` must share exactly one endpoint —
  conservative, deterministic, fires only on clear
  "replacement" relationships. Pairs that drift to
  unrelated `(C, D)` clusters are not supersessions; the
  Phase 87/88 flow handles those as two phases.
- **Linked, not atomic.** Each half is filed as a separate
  proposal on the existing Phase 70 chain (no new proposal
  kind, no chain-schema migration). The
  `supersedes_proposal_id` field on `ProposedPersonaDelta`
  carries the cross-link: the `AppendList`-side points at
  the `RemoveList`-side and vice versa. The operator can
  still approve one half and reject the other (operator
  flexibility); the linkage is **operator-visible context**
  for grouping, not a chain-level atomic primitive.
- **Reuses Phase 87's LLM phraser.** The new facet's prose
  comes from the same `PairPhraser` Phase 87 already uses;
  no second LLM dependency. Per-candidate phrasing failure
  → skip that supersession this cycle (the facet stays via
  the standard Phase 87/88 flow on a future cycle).

Enable it in `~/.config/aivyx/aivyx.toml`:

```toml
[persona_consolidation]
enabled = true
enable_supersession = true   # optional, default false
# ... other Phase 87 knobs ...
```

**What you'll see.** Each supersession produces TWO chain
entries (counted as `filed = 2` on the surface; the
`superseded` counter on `aivyx learning` shows the
supersession event count). The `RemoveList` half's `reason`
cites the new proposal as the replacement; the
`AppendList` half's `reason` cites the old proposal as the
one being superseded. Both appear in the **Proposals** pane
with their normal per-proposal `Revert` actions.

#### Grouped rendering (Phase 94)

Phase 94 closes the first Phase 92 deferral: the CLI and the
Web UI Persona-pane both render linked supersession pairs
as a single grouped unit instead of two unrelated rows.

- **CLI (`aivyx persona proposals`).** Linked pairs render
  with a `└─ supersedes:` indicator under the
  `AppendList`-side row and a `└─ superseded by:`
  indicator under the `RemoveList`-side row. The
  `RemoveList` side always comes first regardless of
  input order. Standalone proposals render byte-identical
  to pre-Phase-94.
- **Web UI Proposals tab.** Linked pairs render as one
  outer card with a `↔ linked supersession (Phase 92)`
  banner header, both halves stacked with a
  `↓ supersedes ↓` arrow between them, and a single
  shared action row: primary **Approve both** + a
  **⋮ Split** menu offering partial actions (`Approve
  RemoveList only`, `Approve AppendList only`, `Reject
  RemoveList only`, `Reject AppendList only`) + **Reject
  both**.

The Web UI **Approve both** action fires two sequential
`ResolvePersonaProposal` IPC calls (RemoveList first,
then AppendList). Phase 92's `each half independently
Revert-able` guarantee covers the half-approved failure
mode without needing a transactional IPC primitive — the
operator finishes via the next refresh.

The grouping is pure client-side rendering: the IPC
contract is unchanged from Phase 92 (the
`supersedes_proposal_id` field on
`PersonaProposalSummary` was already wire-compatible). The
same algorithm runs on both surfaces (Rust helper on the
CLI side, line-for-line JS port on the Web UI side); both
defend the same edge cases (self-reference, dangling
partner id, asymmetric link, same-op pair) by degrading to
standalone rendering.

## Reflection auto-loop (Phase 70)

Phase 70 closes the self-learning half of **P14 Persona**: the
agent observes its own behavior, proposes Persona deltas, and
the operator reviews them asynchronously in a dedicated Web
UI Proposals pane or `aivyx persona proposals` CLI subcommand.

**What's running by default after install:** nothing
auto-reflects. Reflection happens when the agent calls
`reflection.propose` (existing tool, Phase 29). Anything that
fires a reflection turn — operator prompt, mission, or
scheduled `[[schedule]]` block with a reflection-flavored
prompt — produces proposals that land in the new persistent
proposal store and surface in the operator review pane.

**Reviewing proposals:**

```sh
# Terminal:
aivyx persona proposals list                       # status: pending (default)
aivyx persona proposals list --status approved
aivyx persona proposals show <proposal_id>
aivyx persona proposals approve <proposal_id>
aivyx persona proposals reject <proposal_id> --reason "too aggressive"
```

Or open the Web UI at `http://127.0.0.1:7843/` (when enabled)
and click the **Proposals** tab. The pane lets you approve,
reject, or **edit-then-approve** — tweak the proposed op JSON
inline before the daemon applies it. Both the original proposed
op and the operator-applied op are preserved in the proposal
chain for audit (Q3(a) at sign-off).

**Storage isolation.** Pending and resolved proposals live in
a separate encrypted domain (`KeyDomain::PersonaProposals`,
table `aivyx_persona_proposals_v1`) from the approved-delta
persona chain. The HMAC chain uses a distinct genesis seed so
a chain-confusion attack (a Pending row inserted into the
persona chain or vice versa) is structurally rejected at MAC
verification (Q4(a)).

**Pair with Web UI desktop notifications** (Phase 69) for the
tightest review feedback loop: every proposed delta can fire
a `kind = "web-ui"` notification so the browser tab pings the
operator the moment a proposal lands.

**Cron-fired auto-reflection (Phase 71)** runs on the
configured cron. Declare one or more `[[reflection_schedule]]`
blocks in `aivyx.toml`:

```toml
[[reflection_schedule]]
name = "nightly-reflection"
cron = "0 0 23 * * *"          # 11pm daily
lookback_window_secs = 86400   # last 24 hours
```

The daemon spawns a scheduler task at startup (one line per
registered schedule in the boot banner) that fires a reflection
turn at each cron boundary. The reflection turn carries the
canonical reflection prompt plus the lookback-window's
TurnStarted/TurnEnded outcome summaries from the audit chain;
the agent uses `reflection.propose` to record Persona deltas as
Pending rows for asynchronous operator review.

The canonical prompt is intentionally conservative: it tells
the agent to propose only when a pattern recurs in ≥3 distinct
turns within the window, prefer narrower categories
(`BehavioralPreferences`, `LearnedContext`,
`CommunicationAdaptations`) over identity-level changes, and
return empty when no clear pattern emerges. An empty reflection
turn is valid and preferred over speculation.

### Cadence learning — skip-when-idle (Phase 95)

By default the reflection cron fires on every cron boundary
regardless of how much activity happened in the lookback
window. Phase 95 adds opt-in **skip-when-idle**: when
`skip_when_idle = true` on a `[[reflection_schedule]]`, the
scheduler reads the audit-chain growth since the last *fired*
cycle for that schedule. If growth is below
`min_audit_entries_to_fire`, the cycle is skipped entirely
(no LLM calls for Phase 87 phrasing / Phase 91 judgment /
Phase 92 supersession — just a log line + a counter bump).

The operator's `cron` remains the **upper bound** on firing
rate. Cadence learning is monotonic-slower-only: the
scheduler can suppress a fire, never schedule one.

```toml
[[reflection_schedule]]
name = "nightly-reflection"
cron = "0 0 23 * * *"
lookback_window_secs = 86400
skip_when_idle = true                # opt-in, default false
min_audit_entries_to_fire = 50       # default 1
```

The first cycle after a daemon boot fires unconditionally
(no prior baseline to compare against). Subsequent cycles
consult audit-growth. The `last_fired_audit_len` cursor is
updated only on actual fires; a long run of skips
accumulates growth until the threshold is crossed and the
next cycle fires.

Validation: `min_audit_entries_to_fire >= 1` is required
when `skip_when_idle = true` (zero would skip every cycle
unconditionally; the loader rejects this at config time).

**What you'll see.** Each skipped cycle logs
`aivyx reflection: schedule "X" — skipped (audit-growth K
below threshold M)`. The `aivyx learning` surface adds a
**Reflection cadence (Phase 95)** block with one line per
schedule that's made cadence decisions:

```
Reflection cadence (Phase 95):
  nightly-reflection: 7 fired, 2 skipped
```

Schedules with both counts at zero (or schedules the
operator hasn't enabled `skip_when_idle` on) are omitted
from the block to avoid noise.

## Uninstall

```sh
# Remove the binary
rm "$(command -v aivyx)"

# Stop and remove the daemon socket/PID (if a daemon is still around)
aivyx daemon stop 2>/dev/null
rm -f /run/user/$UID/aivyx.sock /run/user/$UID/aivyx.pid

# Remove the encrypted store and config (DESTROYS YOUR DATA)
rm -f ./aivyx.toml /tmp/aivyx-store.redb
# Adjust paths to match your config's [storage] path.
```

The encrypted store contains your conversation history, audit
chain, memory, missions, schedules, profile, and persona deltas.
Deleting it is irreversible — there is no cloud backup by
design (PRODUCT.md G6).

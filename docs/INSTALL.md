# Installing Aivyx

This doc covers the full install matrix. For the abbreviated
"Five-minute setup" path, see the [root README](../README.md).

Aivyx ships a single binary, `aivyx`, plus three optional channel
adapters baked into it (CLI, Telegram, Web UI). There are no
hosted dependencies — your binary talks directly to your LLM
provider (Anthropic / OpenAI-compatible / Ollama) and stores
everything locally in an encrypted redb file.

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

   **Faster path with a starter template** (Phase 66):
   `aivyx init --list-templates` to discover available starters
   (`coder`, `researcher`, `personal`), then
   `aivyx init --template <name>` to run the wizard with
   pre-filled defaults from the template. The generated
   `aivyx.toml` includes the template's role declarations, MCP
   blocks, and commented-out automation hints. See
   [`docs/TEMPLATES.md`](TEMPLATES.md) for the full template
   reference.

2. **`aivyx`** — auto-spawns the daemon (foreground or
   background depending on flag), drops you into a REPL session,
   and serves the Web UI on `127.0.0.1:7843` if you enabled it.

3. **Visit `http://127.0.0.1:7843/`** for the Web UI: Chat,
   Missions, Audit (with cold-verify), Sessions, Profile,
   Persona tabs.

4. **`aivyx --verify-only`** at any time runs the offline
   HMAC audit-chain verification pass.

For deployment guidance (threat model, what Aivyx defends
against, what it doesn't), read
[`docs/THREAT_MODEL.md`](THREAT_MODEL.md) before exposing the
agent to anything sensitive.

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

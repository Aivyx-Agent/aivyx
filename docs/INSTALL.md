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

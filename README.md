# Aivyx

> A personal autonomous agent platform that runs on your hardware,
> talks to cloud LLMs under your own API key, and never compromises
> privacy or auditability for the sake of a feature.

Aivyx is a Rust-built agent framework whose load-bearing
properties are **capability-based security**, **HMAC-chained
auditability**, and **encryption at rest**. It does not run as a
hosted service. There is no Aivyx-the-company server in your
agent's request path; your API key talks directly to the LLM
provider, your data stays on your hardware, your audit chain is
verifiable offline.

## Status (Chapter K exit, 2026-06-12)

| | |
|---|---|
| Phases shipped | Phase 0 → Chapter K (Cost Governance), plus 13 contract amendments |
| Forward-commitment ledger | **Closed** — all 14 PRODUCT.md commitments (P1–P14) and all 7 goal commitments (G1–G7) shipped; subsequent chapters extend the platform within the locked contract |
| Release pipeline | **Wired, dormant** — cargo-dist + GitHub Actions ready for Linux x86_64/aarch64 + macOS x86_64/aarch64; first published release pending public hosting |
| Workspace crates | 29 |
| Rust tests | 4,464 passing |
| Python conformance tests | 24 passing |
| Clippy warnings | 0 |
| Capability scope bases | 81 |
| Encrypted storage domains | 20 |

The arc to date, by chapter:

- **Phases 0–49 — Commitment surface.** Built out the original
  PRODUCT.md surface: channels, daemon, missions, reflection,
  scheduling, MCP, multi-provider LLM, web UI, multimodal input,
  bundled tools, channel/tool SDKs, tool-process IPC.
- **Phases 50–54 (Chapter A — Foundation Closeout).** Paid down
  the cleanup backlog, added the sandbox layer, resynced docs.
- **Phases 56–60 — Profile + Persona.** The operator-declared
  identity layer (P13) and the reflection-written character
  layer (P14) — Aivyx as a *self-learning AI personal assistant
  with a user-defined Profile and Persona shaped to the
  end-user's use-case*.
- **Phase 61 — Distribution (pipeline ready).** Wired the
  release substrate (CI gates, dist config, four-target matrix);
  publication waits on public hosting.
- **Phases 62–99 — Onboarding + recall self-tuning.** Template
  archetypes (`init --template`), the recall-feedback learning
  loop, helpfulness/co-occurrence ledgers, LLM-judged recall.
- **Chapter B — Tooling (100+).** Tool stats, tool-author
  ergonomics, the in-tree adapter checklist.
- **Chapter C — Operator Onboarding (104+).** Docs-landing and
  paper-cut reduction.
- **Chapter D — Substrate Breadth (105–110+).** Audit export,
  richer IPC surfaces, the threat-model-adjacent hardening.
- **Chapter E — Self-Improvement Loop Deepening (114+).**
  Generalised the learning loop beyond recall to the full
  reflection family.
- **Chapter F — External Productivity Integrations (123+).**
  Operator-OAuth productivity tools as separate per-service
  binaries: Gmail, Google Calendar, Google Drive, Notion,
  Obsidian, n8n — each a sandboxed tool process over the IPC
  bridge.
- **Chapter G — Toolkit.** A multi-tool single-binary bundle
  (`web.search` + `task.*` + `health.check.*`), plus the Discord
  / Slack channel adapters and the voice channel.
- **Phases 172–179 — Correction learning + the autonomous
  loop.** A correction-signal learning loop (the agent notices
  when the operator reworks its answer), now structurally
  detected, LLM-judged (genuine rework vs praise), and
  tool-attributed; and the **Aivyx Ralph loop** — a fully
  autonomous, self-re-arming agent loop over an HMAC-chained
  backlog with iteration / wall-clock / token caps, driver-side
  gate verification, and a cross-iteration progress log.
- **Chapter H — Productize (Phases 180–184).** Closing the
  backend-review gaps that stand between a mature substrate and a
  launchable product: a **secure-by-default sandbox** preset for
  tool processes; a **guided first-launch identity builder**
  (the End User shapes their assistant's Personality + Role,
  optionally LLM-assisted); **`aivyx connect`** — guided in-agent
  OAuth onboarding for the productivity tools; **reminders** (the
  first everyday-PA capability); and **conversational
  skill-teaching** — the End User teaches the agent a skill in
  chat. Together these realize the founding "fully customizable"
  promise: Profile, Persona, Roles, and skills are all
  user-shaped.
- **Chapter I — Interface & Reach (Phases 185+).** Richer frontends
  around the one daemon: a **terminal TUI** (`ratatui`/`crossterm`,
  `aivyx tui`) — streaming chat, scrollback, status bar, and
  multi-view navigation (Chat · Missions · Dashboard · Audit · Tools) —
  quarantined to a dedicated `aivyx-tui` frontend crate so the
  substrate stays dependency-clean.
- **Chapter J — Nonagon: Multi-Agent Teams.** The free-core
  multi-agent capability: a **lead** agent convenes up to **9
  attenuated specialists**, decomposes a mission into a **DAG**,
  delegates, verifies, and synthesizes — all in the **one daemon** on
  the **one HMAC chain**, preserving the single-agent ethos (one lead;
  ephemeral, least-privileged specialists, **NT-02:** specialist `⊆`
  lead). Ships the `aivyx-team` engine, **`aivyx team run "<mission>"`**
  / **`aivyx team roster`**, the live TUI Missions panel, and the first
  **vertical pack** — `aivyx-kitchen`'s Back-of-House Nonagon. See
  [`docs/NONAGON.md`](docs/NONAGON.md) + [`docs/VERTICAL_PACKS.md`](docs/VERTICAL_PACKS.md).
- **Chapter K — Cost Governance.** Dollar visibility and budgets layered
  over the token usage the **one HMAC chain** already records. A new
  `aivyx-cost` crate prices each turn's `TokenUsage` into dollars (built-in
  cloud-model rates + `[pricing.<model>]` overrides; local models are free),
  emits a per-turn **`LlmCost`** audit event, and rolls it into a priced
  report behind **`aivyx cost [--today]`**. Enforcement is opt-in via
  `[budget]` caps (`per_run_usd` / `per_day_usd`, *alert* or *deny*): a
  per-run **dollar cap on the autonomous loop** (surfaced in `aivyx loop
  status`) and a **pre-call budget gate on the interactive / team / voice
  turn loop** that refuses a turn before any model call when the daily cap
  would be busted. Free core — observability + safety, not customer billing.
  See [`docs/COST_GOVERNANCE.md`](docs/COST_GOVERNANCE.md).

## Five-minute setup

Aivyx ships zero hosted dependencies. Today's path is
build-from-source; a prebuilt-binary installer is wired and
waiting on public hosting (see [Release pipeline status](#release-pipeline-status)).

**Phase 66 onboarding fast-path:** after `cargo build --release
--bin aivyx`, run `./target/release/aivyx init --template
coder` (or `researcher` / `personal`) to skip the from-scratch
config and run the wizard pre-filled from a starter archetype.
See [`docs/TEMPLATES.md`](docs/TEMPLATES.md) for what each
template contains. The manual path below is shown for
reference.

```sh
# 1. Install Ollama and pull a model (no API key required)
ollama pull llama3.1

# 2. Build aivyx
git clone <repo-url>
cd aivyx
cargo build --release --bin aivyx

# 3. Drop a minimal config in your CWD
cat > aivyx.toml <<'EOF'
[agent]
provider = "ollama"
model = "llama3.1"

[fs]
root = "/tmp/aivyx-sandbox"

[storage]
path = "/tmp/aivyx-store.redb"

[daemon]
web_ui = true   # enable the localhost-only web UI on :7843

[aivyx]
passphrase = "set-a-real-passphrase"
EOF

# 4. Create the fs sandbox and launch
mkdir -p /tmp/aivyx-sandbox
./target/release/aivyx init    # interactive wizard (or skip if you already wrote aivyx.toml)
./target/release/aivyx         # auto-spawns the daemon, drops into a session
```

Then open `http://127.0.0.1:7843/` in a browser — that's the
Web UI. Type a message in the **Chat** tab. Click **Audit** to
watch events land in the HMAC-chained log; click **Verify chain**
to cold-verify the chain offline.

**Terminal frontends + the Nonagon (Chapter I/J):**

```sh
./target/release/aivyx tui                 # the ratatui terminal UI
./target/release/aivyx team roster         # the default 9-role Nonagon
./target/release/aivyx team run "research the latest on X and draft a summary"
./target/release/aivyx team roster --config crates/aivyx-kitchen/assets/kitchen-boh.toml
```

`aivyx team run` hands the mission to a **lead** agent that decomposes it
into a DAG, delegates to least-privileged specialists, verifies, and
synthesizes — every step on the one HMAC chain. A **vertical pack** swaps in
a domain crew via `--config <pack.toml>` (the kitchen Back-of-House Nonagon
is the worked example). See [`docs/NONAGON.md`](docs/NONAGON.md).

For a config that uses Anthropic or OpenAI instead, see
[`examples/aivyx.toml`](examples/aivyx.toml). For a Telegram
adapter, see [`examples/aivyx-semitrusted.toml`](examples/aivyx-semitrusted.toml).
For the full install matrix, see [`docs/INSTALL.md`](docs/INSTALL.md).

## Release pipeline status

Phase 61 wired the release substrate but did not publish a
release. The pipeline is dormant until public hosting is configured:

- `.github/workflows/release.yml` (cargo-dist-generated) cross-compiles
  for x86_64/aarch64 Linux musl + x86_64/aarch64 macOS on every
  `v*.*.*` tag push.
- `.github/workflows/ci.yml` runs `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` on every push to
  main and every PR.
- `.github/workflows/quality-gate.yml` is the shared reusable
  workflow both CI and release pipelines call.

When the project goes public, cutting a release will be: push a
remote, push a `v0.1.0` tag, the workflow auto-publishes
prebuilt binaries + a one-line shell installer. That handoff is
the deferred Task 7 of Phase 61 and reopens as a focused
micro-phase.

## Architecture at a glance

```
operator
   │
   ▼
 channel adapter (CLI / Telegram / Web UI / third-party)
   │
   │  Unix-socket IPC (mode 0600)
   ▼
 aivyx daemon
   ├── turn loop (capability check → audit → execute → audit)
   ├── HMAC-chained audit log (offline-verifiable)
   ├── encrypted redb store (Argon2id → HKDF → ChaCha20-Poly1305)
   ├── 13 substrate tools + role-gated infrastructure tools
   └── tool process bridge (third-party + productivity tools as subprocesses)
```

Twenty-four crates in the workspace. The substrate core:

| Crate | What it owns |
|---|---|
| `aivyx-core` | `Agent` / `Tool` traits, turn loop, the 13 substrate tools |
| `aivyx-capability` | `Scope`, `CapabilitySet`, `TrustTier`, 75 scope bases |
| `aivyx-crypto` | Argon2id, HKDF-SHA256, ChaCha20-Poly1305 |
| `aivyx-storage` | redb-backed encrypted store, 20 key domains |
| `aivyx-audit` | HMAC-chained audit log, offline verification |
| `aivyx-config` | TOML + env loader with source provenance |
| `aivyx-llm` | `LlmProvider` trait + Anthropic / OpenAI / Ollama impls |
| `aivyx-memory` | `memory.{read,write,forget,gc}` + redb-backed substrate |
| `aivyx-channel` | Daemon, CLI/Local channel, Web UI, mission/schedule/reflection/loop machinery |
| `aivyx-mcp` | MCP client adapter (stdio + SSE) |
| `aivyx-tool` | Tool process IPC bridge + sandbox wrapper layer |

Channel adapters — `aivyx-telegram`, `aivyx-discord`,
`aivyx-slack`, `aivyx-voice`.

Productivity integrations (Chapter F/G — each a sandboxed
operator-OAuth tool process) — `aivyx-gmail`, `aivyx-calendar`,
`aivyx-drive`, `aivyx-notion`, `aivyx-obsidian`, `aivyx-n8n`,
`aivyx-toolkit` (web.search + task.* + health.check.*), with
`aivyx-google-oauth` + `aivyx-auth-cli` providing the shared
OAuth substrate.

## Where to look next

**For operators** wanting to use Aivyx:
- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) — what Aivyx
  defends against, what it doesn't. Read this before deploying.
- [`examples/`](examples/) — worked TOML configs for Ollama,
  Anthropic, and the SemiTrusted (Telegram) tier.

**For contributors** adding channels, tools, or capabilities:
- [`docs/CHANNEL_SDK.md`](docs/CHANNEL_SDK.md) — v0 contract for
  writing a channel adapter (in any language; see
  [`examples/python-channel/`](examples/python-channel/)).
- [`docs/TOOL_SDK.md`](docs/TOOL_SDK.md) — v0 contract for
  writing a tool process (in any language; see
  [`examples/python-tool/`](examples/python-tool/)).
- [`docs/ADAPTER_PATTERN.md`](docs/ADAPTER_PATTERN.md) — checklist
  for in-tree adapters.
- [`docs/DAEMON_IPC.md`](docs/DAEMON_IPC.md) — wire format for
  the daemon's IPC protocol.

**For architects** wanting to understand the design:
- [`DESIGN.md`](DESIGN.md) — locked technical contract (13 amendments)
- [`PRODUCT.md`](PRODUCT.md) — locked product contract (P1–P14)
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — phase-by-phase narrative
- [`docs/PRODUCT_ROADMAP.md`](docs/PRODUCT_ROADMAP.md) — product-shape milestone narrative
- [`docs/`](docs/) — per-phase journals (frozen artifacts)

## Building & testing

```sh
# Pre-commit hook (recommended once per clone)
./scripts/install-hooks.sh

# Full sweep
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Python conformance suites (no daemon required)
python3 -m unittest discover examples/python-channel/tests
python3 -m unittest discover examples/python-tool/tests
```

The pre-commit hook runs `cargo clippy --workspace --all-targets
-- -D warnings` before every commit — the workspace has held at
zero warnings since the Phase 9 hook was wired.

## Contributing

This is a single-operator personal-agent platform by design
(PRODUCT.md P1 + P6). Contributions are welcome via the same
channels any open-source Rust project uses: file an issue,
discuss the shape, send a PR. New channels, new tools, new
provider adapters fit cleanly into the existing SDK surfaces.

Architectural changes that touch DESIGN.md or PRODUCT.md require
a formal amendment under `docs/amendments/` — thirteen have been
filed across the arc; the process is established. (The two
contracts have otherwise held untouched for many phases — a
tracked stability discipline.)

## License & trademark

Code is [MIT-licensed](LICENSE). The "Aivyx" name and associated
branding are trademarked — see [TRADEMARK.md](TRADEMARK.md) for
the brand usage rule (MIT + branded: fork the code freely; don't
call the fork "Aivyx").

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

## Status (v0.2.0 — the Studio, 2026-06-16)

| | |
|---|---|
| Phases shipped | Phase 0 → the complete Studio (Chapters R–Z + Voice), plus 13 contract amendments |
| Forward-commitment ledger | **Closed** — all 14 PRODUCT.md commitments (P1–P14) and all 7 goal commitments (G1–G7) shipped; subsequent chapters extend the platform within the locked contract |
| Release pipeline | **Active** — cargo-dist + GitHub Actions build Linux x86_64/aarch64 (musl) + macOS x86_64/aarch64 on each version tag; latest release is **`v0.2.0`** (early pre-release) via the [shell installer](docs/INSTALL.md#shell-installer-recommended) |
| Studio (web GUI) | **Complete** — every screen live: Command · Missions · Chat · Memory (+ graph) · Settings · Agents · Teams · Documents (browse + edit) · Voice; offline, local-first, served on `:7843` |
| Workspace crates | 32 |
| Rust tests | 4,666 passing |
| Python conformance tests | 24 passing |
| Clippy warnings | 0 |
| Capability scope bases | 83 |
| Encrypted storage domains | 20 |

## Five-minute setup

Aivyx ships zero hosted dependencies. The quickest path is the
one-line shell installer (a prebuilt binary for your platform):

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Aivyx-Agent/aivyx/releases/latest/download/aivyx-cli-installer.sh | sh
```

Prefer to compile? The build-from-source steps below work too.

**Onboarding fast-path:** after `cargo build --release --bin
aivyx`, run `./target/release/aivyx init --template coder` (or
`researcher` / `personal`) to skip the from-scratch config and run
the wizard pre-filled from a starter archetype. See
[`docs/TEMPLATES.md`](docs/TEMPLATES.md) for what each template
contains. The manual path below is shown for reference.

```sh
# 1. Install Ollama and pull a tool-capable model (no API key required)
ollama pull qwen3:8b

# 2. Build aivyx
git clone https://github.com/Aivyx-Agent/aivyx
cd aivyx
cargo build --release --bin aivyx

# 3. Drop a minimal config in your CWD
cat > aivyx.toml <<'EOF'
[agent]
provider = "ollama"
model = "qwen3:8b"

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
**Studio**, the local-first web GUI. It opens on the **Command
Center** dashboard; use the **Chat** tab to talk to the agent,
**Memory** to browse what it's learned, **Documents** to read and
edit files in scope, and **Settings** to adjust access + budgets.
The HMAC audit log (with offline **Verify chain**) lives in the
legacy inspection panes at `/classic`.

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

## Highlights

- **Runs on your hardware, no API key.** The headline path is local inference
  via [Ollama](https://ollama.com) with a zero-config on-ramp — auto
  context-window sizing, a vetted tool-capable model, and `aivyx doctor` to
  confirm the path end to end. Anthropic / OpenAI are optional, under *your*
  key, talking directly to the provider.
- **Secure by construction.** Capability-based scopes + trust tiers bound
  exactly what the agent can reach; every action lands on an **HMAC-chained,
  offline-verifiable audit log**; storage is encrypted at rest
  (Argon2id → HKDF → ChaCha20-Poly1305).
- **Operator-chosen reach.** *You* pick how far the agent reaches —
  **sandbox** / **workspace** / **home** / **full** — as an audited setting
  (`aivyx access`); irreversible filesystem ops are confirm-first.
- **A self-learning identity.** A user-defined **Profile** plus a
  reflection-written **Persona/Soul**, seedable at first launch (by hand or
  *"describe it and the model drafts it"*) and governed through approve / edit
  / reject proposals.
- **Multi-agent teams (Nonagon).** A lead convenes up to **9** least-privileged
  specialists, decomposes a mission into a DAG, delegates, verifies, and
  synthesizes — durable, resumable, on the one HMAC chain; **vertical packs**
  swap in a domain crew.
- **The Studio — a local-first web GUI.** Nine offline, Stitch-styled screens
  served on `:7843`: Command Center, Missions, Chat, Memory (+ knowledge
  graph), Settings, Agents, Teams, Documents (browse + edit), Voice.
- **More frontends, one daemon.** A terminal TUI (`aivyx tui`), the CLI, and
  channel adapters — Telegram, Discord, Slack, voice.
- **Productivity integrations.** Gmail, Calendar, Drive, Notion, Obsidian, n8n,
  plus a web/task/health toolkit — each a sandboxed, operator-OAuth tool
  process.
- **Autonomy with brakes.** An autonomous, self-re-arming loop and a
  non-interactive **headless mode** (refuses-and-aborts at gates, never
  auto-approving), with **cost governance** — per-turn dollar pricing and
  `[budget]` caps.

The full phase-by-phase arc lives in [`docs/ROADMAP.md`](docs/ROADMAP.md); the
recent-release narrative in [`CHANGELOG.md`](CHANGELOG.md).

## Release pipeline status

The release pipeline is **active** on the public repo. The latest
release is `v0.2.0` (early pre-release; `v0.1.0` was the first):

- `.github/workflows/release.yml` (cargo-dist-generated) cross-compiles
  for x86_64/aarch64 Linux musl + x86_64/aarch64 macOS on every
  `v*.*.*` tag push, then publishes a GitHub Release with the
  binaries, checksums, and the one-line shell installer.
- `.github/workflows/ci.yml` runs `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` on every push to
  main and every PR.
- `.github/workflows/quality-gate.yml` is the shared reusable
  workflow both CI and release pipelines call — the release
  short-circuits if the gate fails.

Cutting a release is a single step: `git tag vX.Y.Z && git push
origin vX.Y.Z`, and the workflow publishes the binaries + installer.

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

Thirty-two crates in the workspace. The substrate core:

| Crate | What it owns |
|---|---|
| `aivyx-core` | `Agent` / `Tool` traits, turn loop, the 13 substrate tools |
| `aivyx-capability` | `Scope`, `CapabilitySet`, `TrustTier`, the active scope bases |
| `aivyx-crypto` | Argon2id, HKDF-SHA256, ChaCha20-Poly1305 |
| `aivyx-storage` | redb-backed encrypted store, 20 key domains |
| `aivyx-audit` | HMAC-chained audit log, offline verification |
| `aivyx-config` | TOML + env loader with source provenance |
| `aivyx-llm` | `LlmProvider` trait + Anthropic / OpenAI / Ollama impls |
| `aivyx-memory` | `memory.{read,write,forget,gc}` + redb-backed substrate |
| `aivyx-channel` | Daemon, CLI/Local channel, Web UI, mission/schedule/reflection/loop machinery |
| `aivyx-ipc` | wasm-clean wire protocol shared by the daemon and the Dioxus web client |
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

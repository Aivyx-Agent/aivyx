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

## Status (Phase 61 exit, 2026-05-13)

| | |
|---|---|
| Phases shipped | 61 (Phase 0 → Phase 61, plus 10 contract amendments) |
| Forward-commitment ledger | **Closed** — all 14 PRODUCT.md commitments (P1–P14) and all 7 goal commitments (G1–G7) shipped |
| Release pipeline | **Wired, not yet firing** (Phase 61) — cargo-dist + GitHub Actions ready for Linux x86_64/aarch64 + macOS x86_64/aarch64. First published release pending public hosting. |
| Workspace crates | 12 |
| Rust tests | 1074 passing |
| Python conformance tests | 24 passing |
| Clippy warnings | 0 |
| Capability scope bases | 44 |
| Encrypted storage domains | 10 |

The 61-phase arc divides into three halves: **Phases 0–49** built
out the original PRODUCT.md commitment surface (channels, daemon,
missions, reflection, scheduling, MCP, multi-provider, web UI,
multimodal input, bundled tools, channel/tool SDKs, tool process
IPC). **Phases 50–54 (Chapter A — Foundation Closeout)**
finished the deferred refinements, paid down the cleanup
backlog, added the sandbox layer, and brought the documentation
back in sync with the implementation. **Phases 56–60 (Profile +
Persona arc)** delivered the operator-declared identity layer
(P13) and the reflection-written character layer (P14), shaping
Aivyx into a *self-learning, self-improving AI personal
assistant with a user-defined Profile and Persona based on the
end-user use-case*. **Phase 61 (Distribution — Pipeline Ready)**
wired the release substrate (CI gates, dist config, four-target
matrix, install script generation) but holds the first published
release until public hosting goes live — a follow-up micro-phase
will cut v0.1.0.

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
   ├── 8 substrate tools + role-gated infrastructure tools
   └── tool process bridge (third-party tools as subprocesses)
```

Twelve crates in the workspace:

| Crate | What it owns |
|---|---|
| `aivyx-core` | `Agent` / `Tool` traits, turn loop, the 8 substrate tools |
| `aivyx-capability` | `Scope`, `CapabilitySet`, `TrustTier`, 43 scope bases |
| `aivyx-crypto` | Argon2id, HKDF-SHA256, ChaCha20-Poly1305 |
| `aivyx-storage` | redb-backed encrypted store, 9 key domains |
| `aivyx-audit` | HMAC-chained audit log, offline verification |
| `aivyx-config` | TOML + env loader with source provenance |
| `aivyx-llm` | `LlmProvider` trait + Anthropic / OpenAI / Ollama impls |
| `aivyx-memory` | `memory.{read,write,forget,gc}` + redb-backed substrate |
| `aivyx-channel` | Daemon, CLI/Local channel, Web UI, mission/schedule machinery |
| `aivyx-telegram` | Telegram channel adapter |
| `aivyx-mcp` | MCP client adapter (stdio + SSE) |
| `aivyx-tool` | Tool process IPC bridge + sandbox wrapper layer |

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
- [`DESIGN.md`](DESIGN.md) — locked technical contract (7 amendments)
- [`PRODUCT.md`](PRODUCT.md) — locked product contract (P1–P12)
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
a formal amendment under `docs/amendments/` — eight have been
filed across the 54-phase arc; the process is established.

## License & trademark

Code is [MIT-licensed](LICENSE). The "Aivyx" name and associated
branding are trademarked — see [TRADEMARK.md](TRADEMARK.md) for
the brand usage rule (MIT + branded: fork the code freely; don't
call the fork "Aivyx").

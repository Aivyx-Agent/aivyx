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
| First published release | **v0.1.0** (Phase 61) — prebuilt binaries for Linux x86_64/aarch64 + macOS x86_64/aarch64 |
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
end-user use-case*. **Phase 61 (Distribution)** cuts the first
published release with prebuilt binaries — the first phase past
the closed forward-commitment ledger, addressing the largest
adoption-shape gap.

## Five-minute setup

Aivyx ships zero hosted dependencies and one prebuilt binary per
platform. The fastest path is a one-line installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/AivyxDev/aivyx/releases/latest/download/aivyx-channel-installer.sh \
  | sh

aivyx init    # interactive wizard: picks provider, paths, Profile
aivyx         # auto-spawns the daemon and drops you into a session
```

`aivyx init` detects a local Ollama install (no API key needed)
or walks you through an Anthropic / OpenAI key. With `web_ui =
true` in the generated config, open `http://127.0.0.1:7843/` in a
browser — click **Chat**, **Missions**, **Audit** (chain
verification), **Profile**, or **Persona**. The default in-CLI
REPL works without a browser.

**macOS first launch (Gatekeeper).** Unsigned binaries are
quarantined by default. Either right-click → Open the binary
once, or strip the quarantine attribute:

```sh
xattr -d com.apple.quarantine "$(command -v aivyx)"
```

**No native Windows binary in Phase 61.** Use WSL2 for now;
native Windows support is a deferred follow-up phase (daemon IPC
needs a NamedPipe port from Unix sockets).

**Build from source** is still supported for contributors and
unsupported platforms — see [`docs/INSTALL.md`](docs/INSTALL.md)
for the matrix. For a non-Ollama provider config, see
[`examples/aivyx.toml`](examples/aivyx.toml); for a Telegram
adapter, [`examples/aivyx-semitrusted.toml`](examples/aivyx-semitrusted.toml).

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

# Changelog

All notable changes to Aivyx are recorded here. This project adheres to
[Semantic Versioning](https://semver.org). Dates are ISO-8601.

## 0.1.0 — first public pre-release (2026-06-13)

**This is an early pre-release.** Aivyx is a capable, actively-developed personal
agent, but `0.1.0` is its first published binary and has not yet had external
users. Expect rough edges, breaking changes between releases, and gaps in the
docs. Try it, file issues — but don't depend on it for anything critical yet.

### What Aivyx is

A local-first, capability-secured personal AI agent that runs as a daemon on
**your** machine. Its headline path is **"runs on your hardware, no API key"**
via [Ollama](https://ollama.com) — cloud providers (Anthropic, OpenAI) are
optional. The agent has a persistent, self-learning Profile/Persona/Soul, an
HMAC-chained audit log of everything it does, and a capability/trust-tier model
that bounds exactly what it can reach.

### Highlights in this release

- **Local-first on-ramp that just works.** Pick Ollama in `aivyx init` and you
  get a working, tool-using first conversation with **zero manual config**:
  context length is auto-detected from the model (no more starved `num_ctx`
  one-token replies), the wizard recommends and offers to download a vetted
  tool-capable model, and `aivyx doctor` verifies the whole path end to end.
- **Operator-chosen access levels.** *You* decide how far the agent reaches —
  from a sandbox, to your home directory, to full-machine access — via
  `aivyx access` and the init wizard. Irreversible filesystem operations are
  confirm-first; remote channels are automatically attenuated.
- **The agent's own workspace.** A private, always-available directory
  (`~/.aivyx/workspace`) the agent uses for its own thoughts, ideas, plans, and
  projects, with optional proactive journaling.
- **Multi-agent teams in the daemon.** Goal → plan → delegated execution with a
  live TUI feed and human approval gates.
- **A browser Mission-Control GUI.** A Rust→WASM (Dioxus) web client that speaks
  the daemon's wire protocol directly — Missions and Chat in the browser.
- **Productivity integrations.** Gmail, Google Calendar, Google Drive, Notion,
  Obsidian, n8n, and a web/task/health toolkit, each as an isolated tool process
  with operator-provided OAuth.

### Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Aivyx-Agent/aivyx/releases/download/v0.1.0/aivyx-cli-installer.sh | sh
```

Then run `aivyx init` to set up your agent. See `docs/INSTALL.md` for building
from source and `docs/LOCAL_FIRST_RUN.md` for the local-model path.

### Platforms

Prebuilt binaries for Linux (x86_64, aarch64; musl-static) and macOS (x86_64,
aarch64). Windows is not yet supported (the daemon currently uses Unix domain
sockets). All platforms can build from source.

# Changelog

All notable changes to Aivyx are recorded here. This project adheres to
[Semantic Versioning](https://semver.org). Dates are ISO-8601.

## [Unreleased]

### Added

- **Tool-call rate limits & quotas (Chapter Throttle).** A third dispatch gate —
  after the capability + role gates, before execute — bounds *how often* tools
  run, the sibling of Chapter K's dollar budgets for call counts. Opt-in
  `[rate_limit]` config sets per-turn-per-tool, per-turn-total, and per-tool
  sliding-window caps with an `alert` (warn + proceed) or `deny` (block) action;
  a throttled call yields a forensically-distinct `ToolOutcome::RateLimited` and
  a dedicated `RateLimited` audit record (separate from capability/role denials).
  Bounds a runaway turn — autonomous loop, Nonagon mission, or interactive — from
  hammering `web.fetch` / `shell.exec`. Uncapped by default, so existing configs
  are unchanged. See [`docs/RATE_LIMITS.md`](docs/RATE_LIMITS.md); closes backend
  audit finding **F2**.

### Fixed

- **Storage-domain count corrected (21, was documented as 20)** — the Phase-183
  `Reminders` `KeyDomain` was never propagated to the docs. Added a
  `key_domain_count_matches_docs` drift-guard test (audit **F7**).

### Security

- **The Studio `/ws` bridge now enforces an `Origin` check** (closes a
  Cross-Site WebSocket Hijacking / DNS-rebinding vector). A loopback bind is
  not a boundary against the browser — WebSockets are exempt from the
  same-origin policy — so a malicious page the operator visited could
  otherwise open `ws://127.0.0.1:7843/ws` and drive the already-unlocked
  daemon, which since the Studio's write screens can change config and the
  filesystem. The `/ws` upgrade is now accepted only when `Origin` is absent
  (a non-browser client, already inside the trust boundary) or exactly matches
  a loopback origin on the bound port; cross-site, rebinding, wrong-port, and
  `null` origins are rejected with `403`. See `THREAT_MODEL.md` §4.11.

## 0.2.0 — the Studio (2026-06-16)

The headline is the **Studio** — the daemon's web GUI grew from two tabs into a
**complete** local-first mission-control surface, one screen per chapter (R–Z
plus Voice), every one live-verified in a real browser with its WASM bundle
committed. The entire screen inventory is now live: Command, Missions, Chat,
Memory, Settings, Agents, Teams, Documents, Voice.

### Added

- **The Studio web GUI is complete.** Every screen is live, offline, and
  Stitch-styled, served from the daemon's embedded bundle on `:7843`:
  - **Command Center** (S) — the default dashboard: stat cards, active missions,
    a live audit-trail feed, agent status.
  - **Memory** (T) — topic rail + entry cards + keyword/semantic search over the
    self-learning memory, **plus a knowledge-graph view** (MG): a real weighted
    graph (nodes = topics, edges = the co-occurrence ledger's pair scores) laid
    out in-WASM with a deterministic force-directed simulation; click a node to
    filter its entries.
  - **Settings** (U) — the **first config-write surface**: edit the access level
    (confirm-first, enforced server-side) and budgets; section-scoped `toml_edit`
    rewrites with a `ConfigChanged` audit and an honest "restart to apply".
  - **Agents** (V) — a direct **Profile** editor plus the self-learned **Persona**
    governance loop (approve / edit / reject proposals, revert deltas), live.
  - **Teams** (Y) — a read-only view of the active **Nonagon** roster.
  - **Documents** (Z) — a **file browser and editor** over the agent workspace and
    the access-scoped `fs_root`: read files, and (DW) edit + save, create
    files/folders, rename, and delete.
  - **Voice** — a `[voice]` config editor with a daemon-side readiness check
    (ASR/TTS model + espeak data: present / missing / unset) and the launch
    command. Voice itself is a host-local CLI loop (`aivyx --channel voice`), so
    the screen configures it rather than doing browser audio.
- **Onboarding Persona/Skills seed (Chapters W–X).** The end user can give the
  agent a starting Persona + Skills — by hand or by **describing it in words**
  (the model drafts it) — at first launch (`aivyx init` → `[persona_seed]`,
  planted on the signed chain at boot iff empty, adopted turn-one) or live from
  the Studio. See [`docs/PERSONA_SEED.md`](docs/PERSONA_SEED.md).

### Security

- **Documents browsing never escapes its root.** `ListDir`/`ReadFile` reuse the
  fs tools' lexical-resolve → canonicalize → `starts_with(root)` guard, so `..`
  and symlink escapes are rejected over IPC; reads are size-capped + binary-aware;
  the `fs` root is exactly the operator-granted access level.
- **Editable Documents is the most safety-sensitive surface, and gated to match
  (DW).** Writes canonicalize the *parent* dir (a new file can't be canonicalized)
  and re-check `starts_with(root)`; `WriteFile` carries an explicit `overwrite`
  flag (no accidental clobber); rename/mkdir refuse existing targets; `DeleteFile`
  is **empty-only, never recursive, and always requires `confirm: true`** (the web
  shows a confirm modal). Writes are atomic (temp-file + rename), and **every
  mutation is recorded as a signed `DocumentMutated` audit event.**

### Fixed

- The `web_asset_lookup` unit test asserted the bundle was *absent*, which has
  been false since the bundle became committed in Chapter R; rewritten for the
  shipped reality. The full workspace test suite is green (4,666 passing).

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

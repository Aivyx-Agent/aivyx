# Changelog

All notable changes to Aivyx are recorded here. This project adheres to
[Semantic Versioning](https://semver.org). Dates are ISO-8601.

## [Unreleased]

Post-0.3.0 chapters. The recall + memory work (Loom, Codex) is **opt-in and
byte-identical by default** — nothing changes for an existing config until you
enable it.

### Added

- **One-switch smart memory (Chapter Synapse).** The whole memory stack above
  (graph-augmented recall + the knowledge-wiki and typed-graph layers + their
  extraction sweeps) was opt-in and spread across ~14 knobs. **`[memory] profile
  = "smart"`** now arms the coherent bundle with one line (explicitly-set knobs
  still win; the default stays `off` ⇒ byte-identical). Plus the end-to-end
  integration proof the layered arc lacked — the real memory + wiki synthesizer
  + graph extractor + `graph.query` + recall fusion, verified to compose into a
  single turn — and an operator live-verify runbook.
- **Typed knowledge graph (Chapter Lattice).** A real, **directed, typed**
  graph the agent extracts from memory: nodes are entities, edges are directed
  `(subject)-[predicate]->(object)` relations (`deploy` —*depends-on*→ `ci`),
  stored in a new encrypted `KnowledgeGraph` domain. The agent can **query** it
  with a new **`graph.query`** tool (a multi-hop typed traversal — "what depends
  on X?", "what did Y cause?"; gated by a new `graph.read` *infrastructure*
  capability base, no P10 amendment); a new **Studio "Graph" screen** browses
  the directed graph; and `recall_graph_typed_weight > 0` lets the typed
  relations steer recall along *meaningful* edges (vs. mere co-occurrence).
  Opt-in via `[graph].enabled` (a periodic extraction sweep); always derived
  from memory; zero new dependencies. **Chapter Lexicon** then gives the graph
  a **controlled relation vocabulary** — a curated set of canonical relation
  types (`depends-on`, `causes`, `part-of`, …) that synonymous predicates fold
  into, so `depends on` / `requires` / `needs` become one edge instead of three
  (applied at extraction + query, with a sweep that merges existing synonyms);
  unknown relations are kept as-is.
- **Knowledge-wiki layer (Chapter Codex).** A derived, browsable layer over
  memory: the agent consolidates each topic's entries into a **`WikiPage`** —
  an LLM-written summary plus co-occurrence **backlinks** — persisted in a new
  encrypted `KnowledgeWiki` storage domain. A new **Studio "Wiki" screen**
  browses the pages (index → summary + clickable backlinks + source-entry
  count) over read-only IPC. Opt-in: `[wiki].enabled` arms a periodic
  stale-page sweep on the maintenance cadence, and `recall_wiki_weight > 0`
  lets a page summary compete in recall as a single high-signal unit. Pages are
  always *derived* — memory stays the source of truth. Zero new dependencies.
- **`web.extract` + `git.commit` (Chapter Forge).** Two new substrate tools:
  `web.extract` returns a page's readable article text (readability over the
  existing `net.fetch` capability), and `git.commit` stages + commits in an
  operator-allowed repo (a new `git.write` capability base, Trusted-tier only,
  confirm-first). The substrate count moves 13 → 15 (**Amendment A13**).
- **`tools.list` runtime tool introspection (Chapter Atlas).** A refinement
  pass over the ~92-tool surface: a runtime `tools.list` tool, a drift-guarded
  [`docs/TOOLS.md`](docs/TOOLS.md) catalog, and a tool-metadata quality guard.
- **Permissive voice (Chapter Timbre).** Swapped the GPL Piper TTS for
  **Kokoro-82M (Apache-2.0)** + an espeak-free MIT G2P, closing the last GPL
  door (`cargo deny check licenses` clean with no copyleft exception).

### Changed

- **Graph-augmented recall (Chapter Loom).** Auto-recall now fuses three signals
  on one ranking via weighted Reciprocal Rank Fusion: **semantic** (vectors),
  **lexical** (a real BM25 scorer, replacing the old substring match), and a
  **multi-hop co-occurrence graph-walk** (the agent's topic-affinity graph,
  promoted from a passive view to an active retrieval signal). Tunable per
  source under `[embedding]` and proven by a `recall@k` eval harness; default
  off ⇒ identical to prior recall.

## 0.3.0 — source-available (BUSL-1.1) (2026-06-19)

**The headline is the license.** Aivyx moves from **MIT** to the **Business
Source License 1.1 (BUSL-1.1)**: the whole public workspace is now
**source-available** — **free for personal, individual, and non-commercial use**,
with a **paid commercial license for business or production use** — and **every
released version auto-reverts to MIT four years after it ships.** BUSL-1.1 is
source-available, *not* OSI "open source," and we no longer call it that.
**v0.2.0 and every prior release remain MIT in perpetuity** — a license can't be
revoked; the relicense applies from this tag forward. See [`LICENSE`](LICENSE),
[`COMMERCIAL.md`](COMMERCIAL.md), and [`docs/LICENSING.md`](docs/LICENSING.md)
(model + FAQ). This release also lands three breadth chapters since the Studio:
Contacts, Genesis, and the Docker appliance (Harbor).

### Changed

- **Relicensed MIT → BUSL-1.1 (Chapter Charter).** `LICENSE` is the full
  canonical BUSL-1.1 (Additional Use Grant = personal/non-commercial; Change
  Date = 4 years per release; Change License = **MIT**, preserved at
  [`LICENSES/MIT.txt`](LICENSES/MIT.txt)). A dependency-license audit (`cargo
  deny check licenses`, all-features) confirmed no copyleft poisons the combined
  work. Every "open source"/"MIT" claim about Aivyx's own code is now
  "source-available under BUSL-1.1" (README, TRADEMARK, and DESIGN.md's
  open-core deliverable via **Amendment A14**).
- **Contributing now requires a CLA** ([`CONTRIBUTING.md`](CONTRIBUTING.md) +
  [`CLA.md`](CLA.md)), accepted via a `git commit -s` sign-off — necessary so the
  free + commercial dual model can lawfully cover contributed code.

### Added

- **Google Contacts (Chapter Contacts).** A new `aivyx-contacts` tool process
  (People API) with six tools over `contacts.read`/`contacts.write`
  (`contacts.search`/`list`/`get`/`create`/`update`/`delete`); connect with
  `aivyx connect contacts`. The fifth Google integration on the substrate
  pattern. See [`docs/CONTACTS.md`](docs/CONTACTS.md).
- **Unified agent creation (Chapter Genesis).** One onboarding flow across CLI
  and web: the Profile drafter is lifted into the daemon (`DraftProfile` IPC)
  and a "Create your agent" flow (Profile → Persona → access) lands in the
  Studio. See [`docs/ONBOARDING.md`](docs/ONBOARDING.md).
- **Docker server appliance (Chapter Harbor).** A `docker-compose` deployment of
  the daemon + Studio in a container (distinct from the desktop local-first
  install), with two opt-in web-exposure knobs (`web_ui_host`,
  `web_ui_allowed_origins`); CI publishes the image to GHCR. See
  [`docs/DOCKER.md`](docs/DOCKER.md).
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

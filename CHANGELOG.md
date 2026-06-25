# Changelog

All notable changes to Aivyx are recorded here. This project adheres to
[Semantic Versioning](https://semver.org). Dates are ISO-8601.

## [Unreleased]

### Security

- **Bumped `quinn-proto` 0.11.14 → 0.11.15** (RUSTSEC-2026-0185): a remote
  memory-exhaustion via unbounded out-of-order QUIC stream reassembly. A
  transitive dependency (via `reqwest`'s HTTP/3 path); a lockfile-only patch
  bump, no API change. `cargo audit` no longer reports any vulnerability.
- **Documented the `aivyx-desktop` GTK3 advisory cluster in `deny.toml`.** The
  native desktop app's webview (tao/wry/webkit2gtk → the frozen gtk-rs GTK3
  bindings, pinned at glib 0.18) pulls 11 `unmaintained` advisories
  (RUSTSEC-2024-0370 + 0411–0420) plus the glib `VariantStrIter` unsoundness
  (RUSTSEC-2024-0429 = GitHub Dependabot alert #6). All are transitive,
  desktop-build-only (the daemon/core/CLI never link them), unfixable until the
  webview ecosystem moves off GTK3, and not exploitable in the thin shell. The
  unmaintained ones are now ignored with documented rationale (restoring
  `cargo deny` to green); the glib unsoundness is recorded as the accepted
  disposition for the Dependabot alert (dismiss as "tolerable risk").

### Changed

- **One `TurnSafety` choke point for the per-turn knobs.** The deadline + cycle
  breaker were applied ad-hoc at ~7 `ConcreteAgent::new` sites — which is exactly
  why three of them drifted and shipped unprotected. Now every agent-construction
  path ends with `TurnSafety::<posture>(…).apply(agent)`: `interactive` inherits
  the operator's `[agent]` config (REPL, voice, daemon, role-switch child),
  `autonomous` forces the breaker floor (team lead + specialists), and the
  standalone remote-channel builders route through `default()`. `SessionConfig` /
  `AgentStackSpec` now carry a single `turn_safety` field instead of two. No
  behaviour change — the wiring just can't drift across sites again.

### Added

- **`aivyx autonomy` — one dial for how autonomous your agent is (Chapter Reins).**
  A new `[autonomy] level` (`manual` → `assisted` *(default)* → `supervised` →
  `autonomous` → `unleashed`) composes the scattered autonomy knobs (access,
  confirm-first, gate policy, loop arming, self-improvement adoption) into named
  tiers, with per-domain `[[autonomy.override]]` exceptions ("autonomous at
  shell, manual on email") and an `[autonomy.auto_approve]` reversible-scope
  allowlist. `aivyx autonomy show` renders the resolved level and the posture it
  expands to; `aivyx autonomy set <level>` rewrites the section
  (`autonomous`/`unleashed` confirm first). The default `assisted` expands to
  today's behavior byte-for-byte, so an absent `[autonomy]` section changes
  nothing. Its **first runtime effect**: a `supervised`/`autonomous`/`unleashed`
  level **arms the autonomous loop** even without an explicit `[loop] enabled`
  (additive — it never disarms an explicitly-enabled loop). Arming only makes the
  loop *available* (a run still needs `aivyx loop start`) and takes effect only
  when a `[loop]` section exists (which carries the iteration/budget caps). The
  remaining dimensions (gate policy, growth) are wired incrementally — see
  `docs/AUTONOMY.md`. No new capability base; a composition front end to
  primitives already enforced. Settable from the **Studio Settings** screen too
  (an "Autonomy" section with a level picker over a new `SetAutonomyLevel` IPC,
  server-side confirm-first on the autonomy-granting levels).
- **`aivyx --headless "<task>"` — one-shot unattended runs from the CLI.** The
  headless execution mode (Chapter H) was reachable over IPC and from the
  operator-absent drivers, but never from the command line. This wires the
  missing entry: it connects to a **running daemon**, submits one turn that
  *refuses* (records the reason on the audit chain) at any approval gate rather
  than parking for an operator, streams the output, and maps the turn's outcome
  onto a **process exit code** for cron / batch / autonomous callers — `0`
  completed, `3` refused-at-a-gate, `1` any other non-completion. No daemon
  running yields a clear "start `aivyx daemon run` first" error (there is no
  in-process fallback — headless relies on the daemon's gate interception). No
  new tool/base/dep; the interactive paths are byte-identical when the flag is
  absent.
- **Small-cycle breaker (`[agent] cycle_detection`).** A loop-safety companion
  to the consecutive-identical breaker (Chapter Bridle): it catches a repeating
  *cycle* of tool calls (`A,B,A,B,…`) that the consecutive counter resets on —
  the one runaway shape that previously ran until the 32-step cap or the 120s
  deadline. A bounded ring of recent call signatures trips when the tail is
  `min_repeats` back-to-back copies of a `2..=max_period` block, reusing the
  existing `Looping` outcome (no new audit surface). **Default-off** (the turn
  loop stays byte-identical); operators arm it with `[agent] cycle_detection =
  true` — or toggle it from the **Studio Settings screen** (a new "Agent" section
  with a Cycle-breaker toggle, over a `SetCycleDetection` IPC that rewrites the
  `[agent]` section; `GetSettings` now reports the current state).
- **Nonagon team agents get the cycle breaker as a built-in safety floor.** The
  lead and every specialist run autonomously inside a mission — no human watches
  each turn to `/cancel` a runaway — so they always get the small-cycle breaker
  (like `MAX_STEPS_PER_TURN` is always on), independent of the interactive
  `[agent] cycle_detection` knob.

### Fixed

- **`[agent]` per-turn knobs now reach the daemon agent and the role-switch
  child** (the Studio + persistent chat path, and role sub-sessions).
  `turn_timeout_secs` and the new `cycle_detection` were applied only on the
  REPL/voice paths (`build_agent_stack`); the daemon agent and the `role.switch`
  child factory built raw `ConcreteAgent`s that skipped them, so they always ran
  the 120s default and no cycle breaker — a latent gap on the primary path, now
  closed (the child inherits the operator's config, like its parent).
- The `channel-voice-full` feature build: the voice `AgentStackSpec` literal had
  drifted from the struct (it predated `turn_timeout`), so it failed to compile
  under that feature. Restored, with the new cycle-detection knob wired in.

## [0.7.0] — 2026-06-24

The desktop & experience release. A native desktop app, a top-to-bottom Studio
overhaul, end-user documentation, and the first ecosystem seam for vertical
packs — all over the unchanged security core (no new P10 amendment, no new
capability surface).

### Added

- **Native desktop app (`aivyx-desktop`).** A thin `tao` + `wry` shell that
  hosts the Studio in a system webview (reusing the exact WASM UI, no port),
  with daemon lifecycle (attach or spawn), a **system tray** (Open Studio ·
  Restart daemon · Start at login · Quit) and hide-to-tray, **native
  approval-gate notifications** (a background WS client watches for missions
  awaiting approval), a **global hotkey** (`Ctrl+Shift+A`) to summon the window,
  and launch-on-login. Packaged with `cargo-bundle` (a `.deb` on Linux, a
  `.app`/`.dmg` on macOS) via a dedicated release workflow. Linux runtime deps:
  `webkit2gtk-4.1`, `libayatana-appindicator`, `xdotool`/`libxdo`.
- **`aivyx-vertical-sdk`** — a thin, semver-stable facade that re-exports only
  the pack-facing slice of the engine, so vertical packs survive core refactors.
  The Kitchen pack is re-pointed at it as the open reference example;
  `crates/verticals-private/` (git-ignored, auto-joined via a workspace member
  glob) is the home for commercial packs.
- **End-user guide** — a task-oriented `docs/guide/` (welcome, getting started,
  the Genesis wizard, per-feature pages, the desktop app, troubleshooting),
  surfaced as a **Guide screen** in the Studio (markdown rendered in-app via
  `pulldown-cmark`) with working cross-page links.
- **Studio command palette** — `Ctrl/Cmd-K` to fuzzy-jump to any screen.
- **Deep-linking** — the active screen is mirrored in the URL hash, so screens
  are bookmarkable/shareable and survive a reload, and back/forward navigate.
- **Contextual help** — a topbar "?" that opens the Guide to the page for the
  current screen.

### Changed

- **Responsive Studio shell** — the sidebar collapses to a hamburger drawer
  below tablet width, the nav is grouped into labeled sections, and the screen
  grids stack on narrow viewports.
- **Loading skeletons** across every data-driven screen (Command Center, Memory,
  Skills, MCP, Teams, Documents) instead of flashing empty/zero states.
- **UI polish** — a distinct icon per nav item, a single daemon-status indicator
  (was duplicated), and a corrected version footer.
- **Windows** is documented as supported via WSL2 or the Docker appliance (no
  native binary yet — the daemon's IPC is Unix-socket-only).

### Accessibility

- Visible `:focus-visible` keyboard focus rings (there were none), a
  skip-to-content link, `main`/`nav` landmarks, and `aria-label`s on
  previously-unlabeled inputs and icon-only buttons.

### Fixed

- The CI quality gate is green again: install the desktop crate's GTK/WebKit
  system deps for the `--workspace` build, and clear a `clippy::type_complexity`
  lint in `aivyx-web`.

## [0.6.0] — 2026-06-21

The toolbox release. Three chapters **widen what the agent can do** without
touching the security model: a pack of exact pure-compute utilities, readers
that turn operator files into legible structured content, and the wiring that
makes the whole MCP server ecosystem an operator-config story. None adds a P10
substrate amendment or a new capability surface beyond what each tier already
allows.

### Added

- **Utilities pack (Chapter Abacus).** Five exact, deterministic helpers a
  language model is structurally bad at doing in its head, in the existing
  `aivyx-toolkit` process: **`calc.eval`** (a hand-rolled, zero-dependency
  arithmetic evaluator — `+ - * / % ^`, parens, `sqrt/abs/round/floor/ceil/
  min/max`), **`convert.units`** + **`convert.time`** (length/mass/temperature/
  volume/digital via a curated table, and IANA-timezone conversion via
  `chrono-tz`), and **`date.diff`** + **`date.add`** (calendar-correct date
  arithmetic). Three group bases (`calc.eval`, `convert.units`, `date.compute`)
  — the **first toolkit surface reachable below the Trusted tier** (SemiTrusted),
  because pure compute touches no network, filesystem, or operator data.
- **Structured-data readers (Chapter Sheaf).** Three tools in the new
  `aivyx-dataread` crate that turn a file the agent can already reach into
  legible structured content — the file analogue of `web.extract`:
  **`data.csv`** (delimited text → rows), **`data.xlsx`** (a spreadsheet's
  binary zip+XML, which `fs.read` cannot expose, via `calamine`), and
  **`data.pdf`** (a PDF's text layer via `pdf-extract`, no OCR). Each **reuses
  the existing `fs.read` capability and sandbox** — it only parses bytes the
  agent could already read, so it adds no new capability base and no new I/O
  reach (infrastructure tier, no P10 amendment).
- **MCP servers that actually work, especially keyed ones (Chapter Conduit).**
  The MCP client could connect to servers but not *authenticate* to them, and a
  misconfigured server failed silently. Conduit adds **`[[mcp_server]] env`**
  (secrets to a stdio child — the GitHub MCP server's
  `GITHUB_PERSONAL_ACCESS_TOKEN` was previously unconfigurable) and
  **`[[mcp_server]] headers`** (e.g. `Authorization: Bearer` to a remote server,
  never overriding protocol-reserved headers), both with **`${VAR}`
  interpolation** resolved from the daemon environment so secrets stay out of
  `aivyx.toml`. Stdio **stderr is now captured** (last 50 lines) instead of
  discarded, and **`aivyx mcp status`** reports each configured server as
  connected (with its tool count) or failed (with the reason and captured
  stderr). No new capability base, P10 amendment, or dependency — the
  GitHub/weather/Google-Tasks integrations are now operator config, not in-tree
  builds.

## [0.5.0] — 2026-06-21

The local tool-calling release. A small local model (an in-process GGUF on the
mistral.rs engine) can now reliably **drive the agent loop to completion** —
emitting a valid, real-named tool call by construction, then finishing its turn
instead of looping. Both chapters are **opt-in and byte-identical by default**;
no new tool, capability base, P10 amendment, or dependency.

### Added

- **Reliable local tool-calling (Chapter Stencil).** Small local models are
  fragile on the agent loop — they hallucinate tool names and emit malformed
  arguments, and four prompt-substrate phases proved it can't be fixed from the
  prompt. Stencil adds the lever the prompt can't reach: **grammar-constrained
  decoding** on the in-process mistral.rs engine. With **`[mistralrs]
  constrain_tool_calls = true`** (default off), the decoder is constrained to a
  JSON-Schema grammar built from the registered tools, so a small GGUF emits a
  valid, *real-named* tool call — or a `respond` text escape — **by
  construction**, not by hoping. Live-proven on Qwen3-4B: it called real
  `fs.read` + `memory.write` with schema-valid arguments where prior phases
  never got it to invoke a write tool at all. No new tool, capability base, P10
  amendment, or dependency; byte-identical when off.
- **Hardened local tool-calling (Chapter Bridle).** The harness that makes
  Stencil's primitive usable in the wild — a grammar that forces a valid call is
  necessary but not sufficient if the model then can't stop calling it (Stencil's
  live run watched a constrained model loop on one tool call until the deadline).
  Three guarded, default-safe fixes: **(A)** a turn-loop **repeated-call breaker**
  that stops a turn after *N* consecutive identical tool calls (default 3) with a
  distinct `[turn stopped: repeated tool call]` outcome — a generalizable safety
  net for any local model, on by default because it only fires on identical
  repeats; **(B)** a constrained-mode **`respond` preamble** that teaches the
  model how to reply in plain text and finish (the root-cause fix for the loop);
  and **(C)** an operator-configurable **`[agent] turn_timeout_secs`** so a
  legitimately slow local backend can complete a turn (unset → the 120s default,
  byte-identical). Live re-verified on Qwen3-4B: the looping scenario now ends
  cleanly — one tool call, a plain-text answer, a completed turn. No new tool,
  capability base, P10 amendment, or dependency.

## [0.4.0] — 2026-06-20

The memory + skills release. Everything below is **opt-in and byte-identical by
default** — nothing changes for an existing config until you enable it. The
agent's memory now compounds (graph-augmented recall + a knowledge-wiki + a
typed knowledge graph, all behind one `[memory] profile` switch), and its
skills now learn (they sharpen when they underperform, the agent authors new
specialized ones from its own knowledge, and a Studio Skills screen makes the
whole repertoire legible). Plus a pre-release cleanup sweep that cleared the
chapter deferrals and stabilized the test suite.

### Added

- **The Skills library (Chapter Repertoire).** A new Studio **Skills** screen
  (the thirteenth) gives the agent's skills a home: every skill — operator-
  taught, agent-authored (Praxis), agent-refined (Whetstone) — with its
  effectiveness (the WH.2 decayed EWMA as a bar + bucket label), provenance
  badge, `domain`, version, and lineage, and the full procedure on demand. A
  banner links pending skill proposals to the Agents screen (where their
  approval already lives). Read-only over a new `GetSkills` IPC; no new
  capability base, tool, or storage domain.
- **Skills authored from knowledge (Chapter Praxis).** The agent now writes its
  own **specialized skills** from what it has learned: on the reflection cadence
  it finds a topic with rich, connected knowledge (a substantial wiki page + a
  typed-graph neighbourhood) but no skill, and proposes a grounded specialized
  skill synthesized from that page + those relations (tagged with the topic's
  `domain`, agent provenance). Like Whetstone's refinements, it's a governed,
  propose-only persona proposal in the existing Agents UI. Opt-in via
  `[skill_authoring]`, off by default; reuses the wiki/graph stores — no new
  agent tool, capability base, or storage domain.
- **Skills that sharpen (Chapter Whetstone).** Skills are no longer a static
  list. Each turn folds its skill outcomes into a decayed per-skill
  effectiveness ledger, and on the reflection cadence the agent **proposes a
  refined version** of an underperforming skill — the operator's *or* its own —
  as a governed supersession pair (provenance + lineage) that surfaces in the
  existing Agents approve / edit / reject UI. Opt-in via `[skill_refinement]`,
  off by default, propose-only; no new agent tool or capability base.
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

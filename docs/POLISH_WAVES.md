# Polish Waves — the decomposed v0.9 backlog (Chapter Vitrine's real output)

> **Status: sub-project 1 done (2026-08-27), 2–7 not started.**
> `V09_PLAN.md` row 4 ("Polish waves —
> fix the Vitrine backlog, batched by screen family") was a one-line
> placeholder that never got its own doc, the way the phase-planning
> convention (`ROADMAP.md`'s "How this document is maintained") says a
> row should once it grows past a paragraph. This is that doc, scoped
> 2026-08-27 directly against `docs/VITRINE.md`'s real, dated findings
> (not inferred, not re-summarized from memory) plus a live recheck of
> what's actually shipped since.

## Why this exists

`V09_PLAN.md`'s sequence table went unmaintained for 7+ weeks after it
was locked (2026-07-04) — Vitrine (row 3) and Fleet panel (row 5) both
shipped without anyone updating it, while `aivyx-ecosystem/ROADMAP.md`
inherited the stale "Vitrine still in flight" framing rather than
checking directly. See `V09_PLAN.md`'s own "table audited 2026-08-27"
note for the full correction. Row 4 — the actual Vitrine backlog — was
never decomposed at all; it's the largest remaining piece of v0.9 and
this document exists so it doesn't repeat that staleness.

**Rows 6–8 are folded in here too** (Governed-write completions,
`/classic` retirement, Schedule-write autonomy gating) — they're part
of the same "make every surface feel finished" backlog and share
sequencing dependencies with the Vitrine-sourced work below, so
tracking them in one place beats splitting across two docs.

**Deliberately excluded, by explicit user decision (2026-08-27):**
Nonagon role-based team templates (the operator's 3rd "product
thought" at the end of `VITRINE.md`) — a genuinely new product feature
(a guided, LLM-drafted "create your own Nonagon" flow), not polish on
an existing surface. Logged as a v1.0-or-later candidate, not part of
this backlog.

## Sequence

Fast/low-risk first, design work late (so it covers the final screen
set once rather than restyling twice), the biggest/riskiest piece last
(so it benefits from the proven write-recipe patterns landing first).

| # | Sub-project | Contains | Size | Status |
|---|---|---|---|---|
| 1 | **Small backlog sweep** | See below | Small each | ✅ done 2026-08-27 (6 of 9 fixed, 1 already done, 1 checked/not reproducible, 1 deferred — see below) |
| 2 | **`/classic` retirement** | See below | Medium | Scoping in progress 2026-08-27 |
| 3 | **Repertoire / governed-write completions** (V09_PLAN row 6) | See below | Small–medium | Not started |
| 4 | **Agent turn-quality fixes** | See below | Medium–large | Not started |
| 5 | **Missions polish** | See below | Medium | Not started |
| 6 | **UI Modernization pass** | See below | Large, design-heavy | Not started |
| 7 | **Config-write surface area** ("credentials in Studio") | See below (incl. V09_PLAN row 8) | Largest | Not started |
| 8 | **Tool/server call-stat observability** | See below | Large | Not started |

Each sub-project gets its own brainstorm → spec → plan cycle when its
turn comes, per the workspace's usual SDD process — items 1's small
items are the exception, matching the established lightweight-execution
precedent (2026-08-26's 10-item cleanup) rather than full ceremony.

---

## 1 · Small backlog sweep — done 2026-08-27

All 9 items resolved one way or another; none deferred silently.
Verified: `cargo clippy --all-targets -- -D warnings` clean across
default-members, full `cargo test` zero failures, throughout.

- ✅ **Already done, found not fixed** — Create-Agent nav placement
  (`VITRINE.md` operator product thought #2). Turned out to already be
  implemented: `aivyx-web/src/main.rs`'s `Sidebar` component has a
  `genesis_done` check dated "Operator ask (2026-07-06)" — the day
  after Vitrine itself — that hides the Create nav entry once a
  Profile exists. Shipped, just never recorded anywhere, same pattern
  as Vitrine/Fleet-panel's own staleness. No code change needed.
- ✅ **Fixed** — Gatehouse token reveal affordance (§13). New `aivyx
  doctor` "Web UI (Gatehouse):" section reveals the configured token
  instead of sending the operator to grep `aivyx.toml`. "Regenerate"
  (a write) stayed out of scope for this read-only command.
- ✅ **Fixed** — `SkillInvocation.session_id` vs `TurnStarted.session_id`
  divergence (§6). Root-caused, not just logged: `IpcChannelBridge::
  session_id()` delegated to the wrapped channel's own session instead
  of parsing the same `sid` the turn's `Message.session_id` was built
  from. Mutation-tested regression test added.
- ✅ **Fixed** — skill/topic naming leak (§6 P3). Praxis-authored skill
  names are now humanized (`overall_condition` → `Overall Condition`);
  `domain` (the internal lookup key) is unchanged.
- ✅ **Checked, not reproducible in current code** — wiki topic-key
  normalization, `operator-note` vs `operator-notes` (§4 watch-item).
  Every "operator-note" (singular) occurrence in the current codebase
  is a `contradiction.rs` test fixture; the one real production
  constant (`memory_recall.rs`'s `EXPLICIT_MEMORY_TOPIC`) is
  "operator-notes" (plural), used consistently. The July-era divergence
  (if real) isn't reproducible against today's code — no fix applied,
  since there's nothing currently broken to fix.
- ✅ **Fixed** — top-1 skill-cosine fuzziness (§5/§6 watch-item). The
  injection log line now includes the runner-up skill's own name+score
  alongside the winner, per the operator's own suggested diagnostic.
- **Deferred, found to be bigger than "small"** — MCP tool-level health
  signal (§10). Needs real audit-chain aggregation (recent per-tool
  success/failure, not just connection-level status) plus a new
  Lantern-screen surface — a real sub-project of its own, not a small
  item. Left for a future pass; not silently dropped.
- ✅ **Fixed** — host-firewall visibility (§0 P2). The non-loopback
  startup warning now names the host firewall explicitly;
  `docs/INSTALL.md` and a new `docs/GATEHOUSE.md` "Known gap" section
  cross-reference the same finding.
- ✅ **Fixed** — rate-limited rejected-token log line (§0 P2). Both 401
  rejection sites in `aivyx-channel/src/web_ui.rs` now log `aivyx web
  ui: rejected token from <ip>`, rate-limited per-IP (boundary-tested)
  rather than once per request.

Commits: `285ac107` (session_id), `2a2726fc` (skill naming),
`2f8283a6` (runner-up logging), `ea6bb3b9` (rate-limited log +
firewall docs), `d92d4746` (doctor Gatehouse hint). Merged to `main`,
pushed.

## 2 · `/classic` retirement (V09_PLAN row 7) — scoping in progress

Real inventory from `VITRINE.md` §11 (not `V09_PLAN.md`'s own "expect:
3 panes" guess): **audit, sessions, notifications, learning**.
Notifications already shipped (Chapter Herald, 2026-07-07) — 3 remain.
Scoped into 5 pieces 2026-08-27, real code investigated (not guessed)
for each:

- **A. Web Audit screen** — port `ListAuditEntries` (paginated,
  `from_seq`/`limit`, server-capped at 500) + chain-verify, the proven
  read-only recipe (same shape as Notifications).
- **B. Web Sessions screen, enriched** (user decision 2026-08-27: not
  a bare port) — `DaemonState.sessions` is a bare `Vec<String>` today
  (`SessionSummary` carries only `session_id`); enrich with channel/
  frontend type, trust tier, `created_at`, and `last_active_at`
  (updated per-turn, not just at `StartSession`) — a real backend
  change to `DaemonState`'s session tracking, not just a new screen.
- **C. Learning → Command Center** (user decision 2026-08-27, matching
  `VITRINE.md`'s own candidate) — a Learning panel/card on the existing
  Command Center screen via `GetLearningInsights`, not a new nav
  destination.
- **D. The actual retirement** — once A/B/C exist, delete the
  `/classic` panes with Studio equivalents from
  `crates/aivyx-channel/src/web_ui_static.html`, replace `/`'s
  no-bundle fallback with a small dedicated page (NOT the multi-pane
  legacy app) — `/classic` currently serves double duty (the legacy
  inspector *and* the emergency fallback when the wasm bundle isn't
  built), so this needs care, not a blind delete.
- **E. TUI Audit view** (user decision 2026-08-27: include TUI, not
  web-only) — `aivyx-tui`'s `View::Audit` is a hardcoded
  `placeholder_lines(...)` today (`render.rs`); wire the same
  `ListAuditEntries` query A uses into a real ratatui rendering with
  pagination keybindings. A genuine second implementation (ratatui,
  not a copy of the Dioxus component) but the same backend query.

**Split out 2026-08-27, not part of this sub-project:** the TUI's
`View::Tools` placeholder ("the registered tools — provenance,
capability scope, and call stats") is not a port — there is no
existing backend query for tool-registry/call-stat data at all. Its
call-stats half needs the same audit-chain aggregation mechanism as
the MCP tool-level health signal (deferred to sub-project 7) — building
that mechanism twice would be wasteful and inconsistent, so both now
live together in **sub-project 8**.

## 3 · Repertoire / governed-write completions (V09_PLAN row 6)

- Studio "Add skill" write UI (Tutor TU.3).
- Repertoire approve-in-place.
- Invocation history in the Repertoire screen.
- No credentials involved — reuses the proven `toml_edit` writer +
  server-side validation + restart-required UX recipe as-is.

## 4 · Agent turn-quality fixes

All backend/prompt-logic, no new screens (one Studio line-rendering
change only):

- **Tool-failure thrash** (§2b P2) — after N consecutive tool-call
  failures, nudge the model to stop and report the outage instead of
  degenerating into off-task fetches.
- **gpt-oss finishing-family gaps** (§2b, two P2s + a P3, same root
  cause): bare tool-args JSON escaping as the final answer, empty
  completions, and a wrong-sub-question answer under heavy fetch —
  detect the family, assign a finishing strategy; add a final-message
  floor ("model produced no usable reply") for empty/malformed
  completions.
- **Studio "(no reply)" rendering** (§2, §2b) — render *something*
  when a turn completes with empty text, instead of a silent void.
- **Identifier-fidelity check** (Candor family — §2b P3 ×2, §3 P3):
  three independent real repros (VH-EZT→VH-EQT, METAR `22012KT`→"220
  kt", the ICAO-transposition family) — flag a reply token that's
  edit-distance-1 from a recalled/tool-provided identifier.
- **Source-currency instinct** (§2b P2) — the GA-airports answer used
  a Wikipedia list mixing 1930s-defunct fields with live ones; needs
  an "is this source current?" check in the evidence-discipline path.
- **Chat tool-call argument rendering** (§2 P3) — Studio chat lines
  show only the tool name; mission/cron turns already journal full
  args (`→ web_search {"query": …}`) — bring chat to parity.
- **Keyed-backend guidance docs** (§2) — DuckDuckGo 202-blocks this
  rig; document Brave/SerpAPI keyed-backend setup for operators hitting
  the same wall.
- **Volunteered-fact persist gap** (§2 P2, half-fixed by Chapter
  Thread) — an answer to the agent's own question now *connects*
  (history replay) but still isn't `memory.write`-persisted; close the
  ask→answered→persist loop.

## 5 · Missions polish

- **Rejected-mission reason display** (§3 P2 — **confirmed still open
  2026-08-27**: `halt_reason` is set once in `aivyx-web/src/main.rs`
  to `None` as a struct default and never read/rendered anywhere;
  Mission Control's new screen did not add this). `TeamMissionRecord`
  already carries the judge's precise verdict — just needs a render.
- **Gate labels with attempt context** (§3, named in the Reprise
  retry-cap fix's own note — "the gate label P3 stands").
- **Handoff-fidelity prompts** (§3 P2 ×2) — specialists confabulate
  file-based handoffs and ignore real data sitting in mission memory;
  plan/member prompts should state where each artifact lives and that
  inputs arrive in the message, not on disk.
- **Mission topic-naming discipline** (§4 P2) — specialists file
  memory under inconsistent conventions (bare-ICAO vs
  `overall_conditions` vs `overall_conditions_summary`) in one mission;
  needs a topic-naming hint or mission-scoped prefix.
- **Contradictory-memory badge** (§4 P2) — Concord already detects
  conflicts (surfaced via `aivyx memory conflicts` CLI only); the
  Memory screen should badge conflicted topics.
- Worth verifying against Mission Control's actual shipped behavior
  before scoping in detail — some Run-feedback gaps described in §3
  (`TeamRunStarted` handling) may be partially superseded by that
  chapter's new live graph view; re-check rather than assume either way.

## 6 · UI Modernization pass

- **Command Center dashboard restyle** (§1 P3, the walkthrough's own
  headline item) — cards read as "flat and boxy, similar to every
  other agentic dashboard"; direction: revisit `aivyx-brand`'s Stitch
  mockups for unrealized design intent, plus fresh research on modern
  dashboard treatments (depth/elevation, gradients/glass, asymmetric
  layout, motion).
- **Memory/Wiki graph-view cleanup** (§4 P3) — "messy and unintuitive"
  per the operator; data is correct, presentation needs layout/
  readability work (clustering, label collision, visual hierarchy).
- **Version-mismatch reload hint** (§1) — a redeployed WASM bundle
  needs one manual reload today; a "new version available — reload"
  hint is a small, natural rider on this pass.
- **Documents markdown rendering** (§8 P3) — render `.md` files as
  markdown (headings, tables, lists, mermaid) instead of raw
  monospace; natural fit since agent deliverables are markdown. Could
  ride this pass or `/classic` retirement's Documents-adjacent work —
  sequence with whichever lands first.
- Sequenced late deliberately: by the time this runs, `/classic`
  retirement (new read-only screens) and the config-write surface area
  below (new Schedules/MCP screens) will have added screens that
  should get the *new* visual language once, not the old one twice.

## 7 · Config-write surface area ("credentials in Studio") — V09_PLAN row 8 folded in

The largest, most architecturally novel piece: the first time Aivyx
handles real credentials (bot tokens, webhook URLs, SMTP creds, MCP
server `env`/`headers`) through a web form rather than hand-edited
TOML. One design problem unlocks three separate operator asks — worth
solving once, deliberately, not per-feature.

- **MCP full lifecycle CRUD** (§10 P2, the operator's own headline
  ask) — add/edit/update/remove `[[mcp_server]]` from the Studio,
  including `${VAR}`-interpolated `env`/`headers` (Chapter Conduit),
  ideally a "test connection" probe before save.
- **MCP tool-level health signal** (§10) — moved to **sub-project 8**
  2026-08-27 (originally landed here from sub-project 1's small-backlog
  sweep, then moved again once sub-project 2's own TUI Tools scoping
  found it shares its core mechanism — audit-chain call-stat
  aggregation — with that item). Still natural to land the Lantern
  screen's own surface alongside the MCP CRUD screen above when the
  time comes; only the aggregation mechanism itself lives in
  sub-project 8.
- **Notify-target CRUD** (Chapter Herald's own explicit deferral) —
  creating/editing Telegram bot tokens, webhook URLs, SMTP creds from
  the web form; today read-only by deliberate decision pending this
  design.
- **Schedules screen** (`VITRINE.md` operator product thought #1) —
  create/edit crons from the Studio for both operator-created routines
  (the proven non-credentialed write recipe) and agent-created ones
  (resolves **V09_PLAN row 8**, the parked `[autonomy]`-dial gating for
  `schedule.create/update/delete` — distinct from the already-shipped,
  security-motivated Recursive-Scheduling Guard, see `V09_PLAN.md`
  row 8's own note). Show per-cron provenance (operator vs agent).
- **Settings coverage expansion** (§9 P2, product) — inventory
  `aivyx.toml`'s operator-relevant knobs and expose them properly;
  reuses the same `toml_edit` recipe already proven by Settings/Teams/
  Roster, just wider surface area.
- **Autonomy-tier confirm dialog** (§9 P3, safety UX) — a dial that
  composes the agent's entire permission posture currently saves as
  casually as any other field; needs an explicit, unmissable
  destructive-action-style confirmation, and arguably a Command Center
  chip for a pending-restart divergence.
- Sequenced last: benefits from the write-recipe patterns proven by
  sub-projects 2, 3, and 6 landing first, and from the new visual
  language sub-project 6 establishes (new screens here should be built
  in it directly).

## 8 · Tool/server call-stat observability

New sub-project, split out of sub-project 2's scoping 2026-08-27. Two
findings that turned out to be the same underlying problem: "is this
tool/server actually working," derived from the audit chain rather
than trusted from connection-time status alone.

- **A shared audit-chain call-stat aggregator** — the piece neither
  finding below has today: recent per-tool (or per-MCP-server) success/
  failure counts derived from real audit entries, not just "did it
  connect at daemon start." Design once, use twice.
- **TUI Tools view** (`VITRINE.md` §12, `aivyx-tui`'s `View::Tools`) —
  currently a hardcoded placeholder ("the registered tools —
  provenance, capability scope, and call stats"); needs a brand-new
  IPC query enumerating the registered `Tool` trait objects
  (`id()`/`name()`/`required_scope()`) plus the shared aggregator's
  call-stat data, rendered in ratatui.
- **MCP tool-level health signal** (`VITRINE.md` §10) — the Lantern
  screen's connection status is connection-level from the last daemon
  start; `web-search` showed green all day while DuckDuckGo silently
  refused its queries. The shared aggregator, surfaced per-server
  instead of per-tool, closes this — natural to land alongside
  sub-project 7's MCP CRUD screen.
- Not yet scoped in detail (found, not designed) — this doc records
  that it exists and why the two findings are joined; a real
  brainstorm/design pass is its own future session.

---

## Deferred, not in this backlog

- **Nonagon role-based team templates** — see "Deliberately excluded"
  above. Revisit as a v1.0-or-later product feature, not a v0.9 polish
  item.
- **Voice work** — `VITRINE.md` §10, operator decision: deferred until
  after v1.0.
- **`aivyx-desktop`'s TUI deeper-pane work** beyond what `/classic`
  retirement covers — `VITRINE.md` §12's own operator product
  direction: pursue Web/TUI/Desktop parity deliberately rather than
  building each surface twice; folds into whichever polish wave above
  touches the relevant screen, not its own separate item.

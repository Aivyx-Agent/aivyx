# Polish Waves — the decomposed v0.9 backlog (Chapter Vitrine's real output)

> **Status: sub-projects 1 (2026-08-27), 2 (2026-08-29), 3 (2026-08-29), and 4 (2026-08-30) done; 5 mostly done 2026-08-31 (4 of 5 items — one reverted at final review, still open); 6–7 not started.**
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
| 2 | **`/classic` retirement** | See below | Medium | ✅ done 2026-08-28 (all 5 pieces A–E shipped — see below) |
| 3 | **Repertoire / governed-write completions** (V09_PLAN row 6) | See below | Small | ✅ done 2026-08-29 |
| 4 | **Agent turn-quality fixes** | See below | Medium–large | ✅ done 2026-08-30 |
| 5 | **Missions polish** | See below | Medium | ⏳ 4 of 5 done 2026-08-31 (topic-naming discipline still open) |
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

## 2 · `/classic` retirement (V09_PLAN row 7) — ✅ done 2026-08-28

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
  **Final-review correction (2026-08-29):** the shipped card is a
  reduced subset of what the legacy `/classic` pane's `loadLearning()`
  rendered, not the same data as originally claimed above — it shows
  `recalls_scored`/`recalls_total`, `promoted`, `proposals_in_window`,
  and `top_helpful` only. The legacy pane additionally rendered
  `persona_selection`, `proactive`, `persona_lifecycle`,
  `accumulated_helpfulness`, `cooccurrence`, and `cluster_recall`. No
  capability was lost — every field is still available via the
  `aivyx learning` CLI command — but the Studio's own GUI surface for
  this data narrowed on the port. Expanding the card to show the full
  field set is real, scoped follow-on work if wanted, not something
  assumed done here.
- **D. The actual retirement — ✅ done 2026-08-28.** With A/B/C in
  place, the `/classic` route was deleted outright from `serve_static`
  in `crates/aivyx-channel/src/web_ui.rs` — until this point it was
  still serving double duty (the legacy multi-pane inspector *and*
  `/`'s no-bundle fallback via the same `HTML` constant), so the
  fallback role was replaced rather than dropped:
  `web_ui_static.html` shrank from a 2,879-line multi-pane app to a
  small dedicated "build the Studio bundle" fallback page that `/`
  still serves when the wasm bundle isn't built. The dangling
  `nav-classic` link in `aivyx-web/src/main.rs` was removed. A new
  test (`classic_route_is_gone`) proves `/classic` now 404s.
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

## 3 · Repertoire / governed-write completions (V09_PLAN row 6) — ✅ done 2026-08-29

Real research (2026-08-29) found 2 of the 3 originally-listed items
already shipped, confirmed against real code in `aivyx-web/src/main.rs`,
not just `docs/REPERTOIRE.md`'s own "done" claim: **approve-in-place**
(`ProposalCard` already renders inline approve/edit/reject for skill
proposals) and **invocation history** (`SkillCard` already shows
`"· invoked {view.invocations}×"` per skill, from the audit-chain-backed
effectiveness ledger). Both were tagged "pre-v0.4.0" in `REPERTOIRE.md`
— shipped long before this v0.9 phase even started; `V09_PLAN.md` row 6
was simply never checked against `REPERTOIRE.md`'s later state.

Only the **Studio "Add skill" write UI** was genuinely missing — closed
by adding a "Teach a skill" form to the Skills screen, wired to Chapter
Tutor's already-working `FrontendMessage::AuthorSkill`/
`SkillAuthorOp::Teach` (already used by the CLI's `aivyx skills teach`;
no backend changes needed at all). **Corrected mechanism assumption**:
this row's own "reuses the proven `toml_edit` writer + ... restart-
required UX recipe" claim was wrong — Chapter Tutor writes directly to
the signed persona chain and takes effect live, no daemon restart. See
`docs/superpowers/specs/2026-08-29-repertoire-teach-skill-design.md`
and `docs/superpowers/plans/2026-08-29-repertoire-teach-skill.md` for
the full account.

## 4 · Agent turn-quality fixes — ✅ done 2026-08-30

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

All 8 items shipped as designed (see
`docs/superpowers/specs/2026-08-29-agent-turn-quality-design.md` and
`docs/superpowers/plans/2026-08-30-agent-turn-quality.md`), then a
real Critical bug surfaced in the final whole-branch review and was
fixed before merge, then closed with a second follow-up plan:

- **Final-review fix wave** (2026-08-30, one combined commit per
  finding-group): the identifier-fidelity check's token filter was
  vacuously true for pure-digit tokens ("3000"/"2025" flagged as
  drifted identifiers on correct arithmetic) and admitted any-length
  all-uppercase acronyms; the tool-failure nudge's own injected text
  leaked into the identifier check's "trustworthy source" pool; no
  upper bound on identifier-token length left an O(n·m) edit-distance
  check unbounded outside the turn's wall-clock deadline; Studio's
  "(no reply)" render overwrote the real reason for every
  non-completed turn (timed out/looping/escalated); the volunteered-
  fact persist gap fix shipped with no length gate (the design's own
  "reuse the recall gate" requirement was dropped during
  implementation) and could be silently disabled by a trailing
  Candor/identifier annotation stripping the trailing "?" its
  question-check relies on. A second review round then found the same
  identifier-filter class of bug once more (a hyphen-only branch was
  still vacuously true for pure-punctuation tokens like markdown table
  separators) — fixed directly.
- **Turn-outcome-correction follow-up** (2026-08-30,
  `docs/superpowers/specs/2026-08-30-turn-outcome-correction-design.md`,
  `docs/superpowers/plans/2026-08-30-turn-outcome-correction.md`) — the
  same final review found an architectural gap: the turn loop's
  post-processing (the reply floor, Candor's claim-check, the
  identifier-fidelity check) never reaches Studio, the TUI, Telegram,
  Discord, or Slack, because those surfaces display raw streamed
  `Text` events and discard the turn's own authoritative `outcome`
  string; only headless was already correct. Closed with two shared
  pure functions (`concat_text_events`, `turn_outcome_correction` in
  `aivyx-ipc`) plus per-surface wiring — deliberately *not* buffering
  the live stream itself (declined during brainstorming as too
  invasive a change to code shared by every channel). That plan's own
  final review then found a Critical regression in the shared helper
  (it assumed a healthy turn's streamed text always equals
  `final_message`, which is false for any multi-step turn where the
  model narrates before a tool call — the common agentic shape — so it
  falsely flagged "⚠ corrected:" and duplicated the answer on ordinary
  successful turns) plus a related bug that duplicated the answer
  behind Candor/identifier annotations instead of showing only the new
  note; both fixed, and a second review round found one more instance
  of the same "wrong marker occurrence" bug (the model's own organic
  text opening a paragraph with a bare "⚠ " could still misfire) —
  fixed. The review also found the daemon-backed CLI REPL (the default
  `aivyx` interactive chat) had the identical gap; added as a 5th task
  and closed in the same pass.
- **Deferred, not fixed on this branch** — the same final review named
  two more surfaces with the identical raw-event-vs-outcome gap:
  **voice** (`aivyx-voice/src/session.rs`) and the **in-process REPL
  fallback** (`aivyx-channel/src/session.rs`). Both hold an in-process
  `TurnOutcome` rather than a formatted string, so they need
  `TurnOutcome::Completed { final_message, .. }` handling rather than
  the string-based helper — a real, scoped follow-up, not a silent
  gap. Also deferred: `format_outcome`'s `Looping`/`MaxStepsExceeded`
  variants drop the turn's own guidance text (`looping_message`/
  `cycle_message`) in favor of a generic reason string, which is now
  the visible ceiling on what the 6 corrected surfaces can show;
  third-party channel-adapter docs (`docs/CHANNEL_SDK.md`,
  `docs/DAEMON_IPC.md`) still describe collapsing everything to `Text`
  without mentioning `outcome` is authoritative, so a new adapter could
  reintroduce this exact gap; Telegram/Discord/Slack replies have no
  length cap (pre-existing, mildly amplified by the correction line);
  and a handful of Minor code-quality notes from the review rounds
  (test-placement cosmetics, a dead post-construction guard, a stale
  charter doc-comment estimate, non-deterministic "closest identifier"
  selection when multiple pool tokens tie, capping the number of
  identifier-drift notes per turn) — none blocking, all small.

## 5 · Missions polish — ✅ 4 of 5 done 2026-08-31, 1 still open

Full account: `docs/superpowers/specs/2026-08-30-missions-polish-design.md`,
`docs/superpowers/plans/2026-08-30-missions-polish.md`. Verified against
Mission Control's actual shipped behavior before scoping in detail, per
the original "worth verifying" note below — the §3 Run-feedback gap
(`TeamRunStarted` handling) was already fixed same-day per `VITRINE.md`'s
own account and Mission Control's later live-graph work never reopened
it; no 6th item was hiding there.

- ✅ **Rejected-mission reason display** (§3 P2). `TeamMissionView.
  halt_reason` already flowed over the wire; `MissionRow` (the plain
  Missions list — Mission Control's own "watchable" set deliberately
  excludes Rejected/Halted missions) now renders it.
- ✅ **Gate labels with attempt context** (§3). `TeamMissionView` gained
  `verify_attempts`; `GateControls` shows "(attempt N)" once a retry has
  happened. **Final-review fix**: the first cut used the wrong threshold
  and was off-by-one (`verify_attempts` counts *failed* verifications,
  capped at `1` by `MAX_MISSION_ATTEMPTS = 2`, so the original `> 1`
  check with a bare `{verify_attempts}` display could never fire in
  production and would have shown "attempt 1" during the actual 2nd
  attempt if it had). Fixed to `>= 1` / `verify_attempts + 1`.
- ✅ **Handoff-fidelity prompts** (§3 P2 ×2). One string in
  `aivyx-team`'s shared `build_input` (used by both the CLI's `aivyx
  team run` and Mission-Control-driven missions) now states plainly
  that upstream context IS the specialist's real input.
- ⏳ **Mission topic-naming discipline** (§4 P2) — **still open.**
  Attempted via a mission-scoped memory-topic prefix (reusing the
  pre-existing, already-enforced `ConcreteAgent::with_memory_topic_
  prefix` mechanism) — implemented, then **reverted at final review**
  once two real problems surfaced: (1) it doesn't actually fix the
  cited finding — a mission's specialists still write 3 inconsistent
  logical topic names, just now isolated per mission instead of
  unified within one; (2) it measurably harmed the *next* item's own
  Concord conflict-detector (needs ≥2 entries under one topic name to
  compare — a mission-fragmented topic space often produces exactly 1),
  plus added unbounded per-mission pages to knowledge-wiki synthesis
  and cluttered the Memory screen's topic rail. **Lesson for the next
  attempt**: a mission-scoped hash prefix isolates missions from each
  other but doesn't make specialists *agree on one name* within a
  mission — the real fix likely needs the LEAD's own plan decomposition
  to hand each step a canonical topic name to use, not a prefix applied
  after the fact.
- ✅ **Contradictory-memory badge** (§4 P2) — scoped up to the full
  resolve/dismiss loop (not just a badge) once research found Concord's
  entire operator loop (`GetMemoryConflicts`/`ResolveMemoryConflict`/
  `DismissMemoryConflict`) already existed and worked, CLI-only. Studio's
  Memory screen now badges conflicted topics and offers "keep this
  one"/"not a conflict" per conflict, matching the CLI's own semantics.
  Known gap, not silently dropped: a successful resolve only re-fetches
  the conflict list, not the currently-open entry list, so a just-
  archived entry stays visible until the operator re-selects the topic
  (`read_task`, where the ack lands, has no access to the Memory
  screen's own local view-scope signal — closing this needs either a
  shared scope signal or a different refresh mechanism).

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
- Sequenced after `/classic` retirement (already shipped) and before
  the config-write surface area below. **Corrected 2026-08-31**: this
  bullet previously claimed the opposite ("sequenced late... by the
  time this runs, the config-write surface area... will have added
  screens"), contradicting both the sequence table above and item
  7's own note ("benefits from... the new visual language sub-project
  6 establishes") — two signals against one stale one. The reconciled
  reading: this pass establishes the new visual language now, and
  item 7's new Schedules/MCP/notify-target screens are built directly
  in it, never restyled.

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

# Polish Waves — the decomposed v0.9 backlog (Chapter Vitrine's real output)

> **Status: sub-projects 1 (2026-08-27), 2 (2026-08-29), 3 (2026-08-29), 4 (2026-08-30), 6 (2026-08-31), and 7 (2026-09-02) done; 5 mostly done 2026-08-31 (4 of 5 items — one reverted at final review, still open). Sub-project 8 not started.**
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
| 6 | **UI Modernization pass** | See below | Large, design-heavy | ✅ done 2026-08-31 (all 4 items, 7 tasks — see below) |
| 7 | **Config-write surface area** ("credentials in Studio") | See below (incl. V09_PLAN row 8) | Largest | ✅ all 3 plans shipped 2026-09-01/02 (architecture + MCP CRUD; notify-target + channel-adapter CRUD; Settings coverage expansion); 2 items dropped as already-shipped |
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

## 6 · UI Modernization pass — done 2026-08-31

All 4 findings shipped, as 7 tasks on branch `ui-modernization`
(merged to main). Design spec:
`docs/superpowers/specs/2026-08-31-ui-modernization-design.md`. Plan:
`docs/superpowers/plans/2026-08-31-ui-modernization.md`.

- ✅ **Command Center dashboard restyle** (§1 P3) — root cause was
  precise, not stylistic: `--shadow-md`/`--shadow-ambient` were
  already-defined brand-spec tokens (`brand-guidelines.md`'s own
  Shadow System table assigns them to "Cards"/"Page sections"
  respectively) that were simply never applied to `.glass-card`/
  `.panel` — a CSS-only fix, app-wide by construction since every
  screen already uses those classes. Also added the existing
  `--shadow-glow` hover treatment to stat cards (final review caught
  a real regression here — see below).
- ✅ **Version-mismatch reload hint** (§1) — shipped with a corrected
  mechanism from the one the spec originally proposed: comparing
  `aivyx-web`'s own crate version (`version.workspace = true`, rarely
  bumped) would almost never have fired. Instead the WS bridge
  (`aivyx-channel/src/web_ui.rs`) mints a random `boot_id` once per
  daemon process and sends it via a new, bridge-only
  `DaemonEnvelope::ServerInfo` message; Studio compares it across
  reconnects and shows a dismissible banner when it changes — a
  direct proxy for "this daemon isn't the one I started against."
- ✅ **Memory/Wiki graph-view cleanup** (§4 P3) — root cause was label
  collision (no avoidance at all) in the one `compute_layout` engine
  both `MemoryGraph`/`LatticeGraph` already shared, not the layout
  math itself. Fixed with a shared `label_sides` collision-avoidance
  pass, hover-only label disclosure past 25 nodes, and pan/zoom on
  both SVG canvases.
- ✅ **Documents markdown rendering** (§8 P3) — ships with mermaid
  diagram support (scoped in during brainstorming, not originally
  planned). The real finding here was a genuine content-injection gap
  in the natural implementation path: reusing the existing
  `guide::render` (used by the Guide screen) verbatim would have
  passed raw HTML from untrusted content (agent-written or filesystem
  files) straight through `dangerous_inner_html`. A new, separate
  `render_untrusted_markdown` closes it — and needed a 2nd round: the
  first shipped version filtered raw HTML/InlineHtml events but not
  link/image destination schemes, so `[x](javascript:...)` still
  passed through as a live `href` (**caught only at final
  whole-branch review**, fixed and independently re-verified against
  35 crafted bypass attempts before merge). Mermaid.js is vendored
  (not CDN-loaded, matching the local-first stance) and lazy-loaded
  only when a file's rendered preview actually contains a mermaid
  fence.

**Two more real defects surfaced only at final whole-branch review**
(on top of the injection gap above), both cross-task interactions no
single task's narrow review scope could see: mermaid diagrams didn't
re-render after a Source→Preview toggle (a conditionally-called
`use_effect` reading no reactive signal, so it fired once per mount);
and the knowledge-graph's O(n²) force-directed layout was recomputing
on every pan/hover-triggered render instead of only when the
underlying data changed. Both fixed and re-verified before merge.

**Known remaining Minor gaps, not fixed in this branch** (all
confirmed non-blocking, logged here rather than silently dropped):
the reload-hint banner's copy ("new version available") slightly
overstates what `boot_id` actually detects (a daemon restart, which
usually but not always means a new bundle); mermaid's `securityLevel`
relies on the vendored library's own default (`'strict'`) rather than
being pinned explicitly at the call site; the mermaid-fence detection
heuristic can false-positive on literal text mentioning the class
name (wasted local fetch only, no security impact); graph pan/zoom
has no reset-to-fit control and zooms from the viewBox origin rather
than the cursor; a drag starting/ending on a graph node still fires
its click/select handler.

## 7 · Config-write surface area ("credentials in Studio") — V09_PLAN row 8 folded in

The largest, most architecturally novel piece: the first time Aivyx
handles real credentials (bot tokens, webhook URLs, SMTP creds, MCP
server `env`/`headers`) through a web form rather than hand-edited
TOML. Design:
`docs/superpowers/specs/2026-08-31-config-write-surface-design.md`.
Split into multiple implementation plans (a shared architecture is the
real "one design problem" the original framing pointed at; each
consumer screen is its own plan, sequenced so the first proves the
primitive against a real screen before the rest reuse it).

Two of the six originally-listed items resolved before any plan was
written, during brainstorming — grounded against real code, not the
tracking doc's own prose:
- **Autonomy-tier confirm dialog** (§9 P3) — already fully shipped
  (Chapter Reins). Dropped, nothing to build.
- **Schedules screen / V09_PLAN row 8** (`schedule.create/update/
  delete` autonomy-dial gating) — already fully shipped. Studio
  already has full schedule creation (Chapter Chime) with Update/
  Delete wired in the UI; `schedule.update`/`.delete`'s tools already
  enforce own-schedules-only authority (`created_by != Agent` rejects,
  regardless of autonomy tier) plus a `MessageOrigin::System` block,
  and `schedule.update` has full growth-tier-aware re-approval —
  confirmed by reading `schedule_tool.rs` in full (an earlier planning
  pass had wrongly concluded these were missing, based on a flawed
  grep). Dropped, nothing to build.

- ✅ **Plan 1 — Config-write architecture + MCP full lifecycle CRUD**
  (§10 P2, the operator's own headline ask) — shipped 2026-09-01.
  Extends `aivyx-config`'s existing single-section `toml_edit` write
  recipe (Chapter U) to array-of-tables (`[[mcp_server]]`), plus a
  `RedactedSecret` wire-type foundation for the later plans that
  actually carry raw secrets. Add/edit/remove `[[mcp_server]]` entries
  from Studio (transport-specific fields, `${VAR}`-interpolated `env`/
  `headers`) and a "test connection" probe reusing the real,
  boot-time `aivyx-mcp` connection logic. **Final whole-branch review
  found and fixed 2 Critical issues before merge**: the config-read
  path was sending the daemon's fully `${VAR}`-*resolved* env/header
  values to the browser, which Save then baked back into plaintext
  `aivyx.toml` on a single click, permanently destroying the
  placeholder (fixed with a raw-TOML, non-interpolating reader); and
  the array-of-table upsert was silently deleting `[mcp_server.
  sandbox]`/`bundled` on every edit, removing a documented
  `THREAT_MODEL.md` control (fixed by preserving unknown keys — which
  then needed a *second* fix round when preservation turned out to
  carry a stdio-only `sandbox` block onto a switched transport,
  reproducing the exact daemon-won't-boot failure class a sibling fix
  had just closed). Also fixed: a stale-form-state bug in the Add/Edit
  UI, a probe with no timeout that could hang a whole Studio
  connection, and a swallowed post-write reload failure. Full account:
  `docs/superpowers/plans/2026-09-01-config-write-mcp-crud.md`.
  **Incidental discovery, unrelated to this plan's own tasks**: sub-
  project 6's own `dist/` rebuild commit predated that branch's final-
  review fix commit (the Documents-markdown XSS fix among others) —
  `main`'s committed Studio bundle was serving pre-fix code from that
  merge until this plan's own routine `dist/` rebuild corrected it.
  Logged as a process gap (`cargo test` doesn't exercise `dist/`, so
  staleness there is invisible to normal verification), not a defect
  of either branch's real work.
- ✅ **Plan 2 — Notify-target + channel-adapter CRUD** (Chapter
  Herald's own explicit deferral) — shipped 2026-09-02.
  Creating/editing Telegram/Discord/Slack bot tokens, `[[notify_
  target]]` entries (webhook URLs, SMTP creds via the shared `[email]`
  block); previously read-only. Reused plan 1's array-of-table
  primitive (`[[notify_target]]`, plus new per-kind validation and an
  at-most-one-default rule) and shipped a new **partial-update**
  pattern for the 4 singleton sections (`[email]`/`[telegram]`/
  `[discord]`/`[slack]`) — every write field `Option<T>`, `None` means
  leave that value untouched — the first real consumer of
  `RedactedSecret` (built ahead of need in plan 1). **Final
  whole-branch review found a real Critical, then two more rounds each
  found one more adjacent gap in the same validation code — a pattern
  worth naming plainly**: the Studio email card originally sent
  password-only writes, and the loader's all-or-nothing `[email]`
  rule then refused to boot the daemon at next restart with no error
  and no in-Studio recovery (Studio itself served by the dead daemon).
  Fixed by validating the *merged* post-write state and expanding the
  card to expose `host`/`port`/`username`/`from`. The first re-review
  then found two more reachable daemon-bricking gaps the first fix
  left open in the same functions — an unvalidated `tls_mode` value,
  and an `[email]`-presence check that tested the TOML header's mere
  existence rather than whether any of its 6 fields were actually set
  (a bare/commented-out `[email]` header — a normal hand-edit — passed
  the check but still bricked the daemon). A second fix round closed
  both, verified by a second independent re-review that read the
  loader's entire validation function end-to-end specifically hunting
  for a fourth gap and found none remaining in the email path (only
  Minor doc-comment defects and one pre-existing, UI-unreachable gap:
  `chat_id` validation not trimming whitespace, unlike the loader).
  Full account: `docs/superpowers/plans/2026-09-01-notify-target-
  channel-crud.md`.
- ✅ **Plan 3 — Settings coverage expansion** (§9 P2, product) —
  shipped 2026-09-02. `[memory] profile` (a 3-way `off`/`lite`/`smart`
  picker — the earlier spec's "lite/smart" shorthand undercounted it),
  `[embedding]` (`base_url`/`model`/`api_key`), `[proactive]`
  (`enabled`/`target`/`max_per_window`/`window_secs`, target picker
  sourced from plan 2's notify-target list), and `[[reflection_
  schedule]]` CRUD (a new "Reflection schedules" section in the
  Schedules screen — another array-of-table consumer of plan 1's
  primitive, first Studio surface these have ever had, not even
  read-only before). **Final whole-branch review found 2 Critical +
  2 Important, all in the same intersection-of-correct-pieces shape
  this sub-project keeps producing**: (1) the new `[proactive]
  target`-exists check (deliberate defense-in-depth beyond the loader)
  tested presence, not usability — a target disabled via the
  already-shipped Notify-targets screen still passed the check, then
  silently failed at dispatch forever, and the picker itself offered
  disabled targets as if they were fine; (2) the new reflection-
  schedule edit form opened in "daily" builder mode unconditionally,
  so editing ANY existing entry (even just to toggle `enabled`)
  silently rewrote its cron to `0 0 9 * * * *` on Save, regardless of
  what was actually stored; (3) the lookback-hours field truncated
  sub-hour precision on load and silently substituted 24h on invalid
  input, regressing a rule plan 2 established for exactly this class
  of bug; (4) same bug family as (1) — nothing re-validates
  `[proactive].target` if its referent is later disabled or deleted
  through the Notify-targets screen. Fixed (1)-(3); (4) documented as
  a deliberate, out-of-scope deferral (would need a boot-time or
  delete-time cross-check touching plan 2's already-shipped code, not
  a gap this fix wave silently ignored). Independent re-review
  confirmed all 4 genuinely fixed — including checking the loader's
  real `enabled`-default semantics directly rather than assuming the
  fix's `unwrap_or(true)` was right — with no adjacent gap in any of
  the three hunted failure classes, the first time in this sub-project
  a fix wave closed clean on the first re-review. Full account:
  `docs/superpowers/plans/2026-09-02-settings-coverage-expansion.md`.
- **MCP tool-level health signal** (§10) — moved to **sub-project 8**
  2026-08-27 (originally landed here from sub-project 1's small-backlog
  sweep, then moved again once sub-project 2's own TUI Tools scoping
  found it shares its core mechanism — audit-chain call-stat
  aggregation — with that item). Still natural to land the Lantern
  screen's own surface alongside the MCP CRUD screen above when the
  time comes; only the aggregation mechanism itself lives in
  sub-project 8.
- Sequenced last: benefits from the write-recipe patterns proven by
  sub-projects 2, 3, and 6 landing first, and from the new visual
  language sub-project 6 establishes (new screens here are built in it
  directly).

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

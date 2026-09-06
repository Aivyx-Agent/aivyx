# FRONTEND.md Screen Inventory Reconciliation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `docs/FRONTEND.md`'s stale, 13-row Studio screen inventory table with a complete, grouped, 23-row table (plus a footnoted 24th conditional item) that matches the real code.

**Architecture:** A single documentation edit in one file — no code changes. The new table adds a `Group` column mirroring the real `Sidebar` component's own grouping and item order (`crates/aivyx-web/src/main.rs`), so the doc's structure itself makes future drift visible.

**Tech Stack:** Markdown only. Verification is direct code inspection (grep/read), not automated tests — there is no test runner for documentation content.

## Global Constraints

- The `Group` column's order must exactly match the real `Sidebar` component's group order in `crates/aivyx-web/src/main.rs` (—, Workspace, Knowledge, Agent, System), and within each group the row order must match that group's real item order.
- Every "Maps to" cell's wording must trace to an existing source (a `View` enum doc comment, `AuditPanel`'s doc comment, or an existing `docs/ROADMAP.md` line) — nothing invented.
- The 13 already-correct rows' "Maps to" text and chapter labels are unchanged, except Voice's **State** column (🔨 In progress → ✅ Live).
- No code changes — `docs/FRONTEND.md` is the only file touched.

---

### Task 1: Replace the stale table with the reconciled version

**Files:**
- Modify: `docs/FRONTEND.md:85-104`

**Interfaces:**
- Consumes: nothing (first and only task).
- Produces: nothing consumed by a later task (first and only task).

- [ ] **Step 1: Confirm the exact current section to replace**

Read `docs/FRONTEND.md` lines 83-104. It must read exactly:

```markdown
---

## 3. Studio screen inventory (mapped to daemon capabilities)

| Nav item | Maps to | State |
|---|---|---|
| **Command** | dashboard: stat cards + active missions + live audit-trail feed + agent status | ✅ Live (Ch. S — the default landing view) |
| **Missions** | `team.run` goal→plan→gated execution (Nonagon, Ch. L) | ✅ Live, reskinned |
| **Mission Control** | one active mission's live LEAD/specialist graph, click-to-drill-in (current step, capability scopes, NT-02 hint), and abort/pause/resume controls | ✅ Live (Ch. Mission Control) |
| **Chat** | single-agent turn loop + streamed events + gate | ✅ Live, reskinned |
| **Teams** | the Nonagon roster: team header + member cards (role / trust / scopes / tools / soul) — see §10 | ✅ Live (Ch. Y) |
| **Agents** | persona / soul / profile editor: direct Profile write + persona-governance loop (proposals + revert) — see §9 | ✅ Live (Ch. V) |
| **Memory** | self-learning memory browser: topics + entries + search (T) **+ knowledge graph** — see §13 | ✅ Live (Ch. T + MG) |
| **Wiki** | knowledge-wiki browser: synthesized per-topic pages (LLM summary + co-occurrence backlinks + source-entry count) over read-only IPC | ✅ Live (Ch. Codex) |
| **Graph** | typed knowledge-graph view: entity nodes + directed, predicate-labeled relation edges (force-laid-out) over read-only IPC; distinct from the Memory co-occurrence graph | ✅ Live (Ch. Lattice) |
| **Skills** | the skill library: every skill (operator-taught / agent-authored / agent-refined) with its WH.2 effectiveness, provenance, `domain`, version, lineage + the procedure on demand; pending proposals link to Agents; read-only `GetSkills` IPC | ✅ Live (Ch. Repertoire) |
| **Documents** | file browser + **editor** over the agent workspace + the access-scoped fs_root — see §11, §14 | ✅ Live (Ch. Z + DW) |
| **Settings** | the first config **write** surface: access level + autonomy level (both confirm-first) + budgets editable; provider/model read-only — see §8 | ✅ Live (Ch. U, + Reins) |
| **Voice** | `[voice]` config editor + readiness check + launch command (audio runs host-side) — see §12 | 🔨 In progress (Ch. Voice) |

The reference mockups for the locked look: `aivyx-brand/assets/stitch/`
`aivyx_command_center`, `aivyx_missions_orchestration`, `the_terminal`.
```

If the file has drifted from this (different line numbers, different text), stop and report — do not proceed with a mismatched replacement.

- [ ] **Step 2: Replace lines 85-104 with the reconciled table**

Replace everything from `## 3. Studio screen inventory...` (line 85) through the `` `aivyx_command_center`, `aivyx_missions_orchestration`, `the_terminal`. `` line (line 104) — i.e. everything except the leading `---` on line 83 and the blank line on line 84, which stay as-is — with:

```markdown
## 3. Studio screen inventory (mapped to daemon capabilities)

| Group | Nav item | Maps to | State |
|---|---|---|---|
| — | **Command** | dashboard: stat cards + active missions + live audit-trail feed + agent status | ✅ Live (Ch. S — the default landing view) |
| Workspace | **Chat** | single-agent turn loop + streamed events + gate | ✅ Live, reskinned |
| Workspace | **Missions** | `team.run` goal→plan→gated execution (Nonagon, Ch. L) | ✅ Live, reskinned |
| Workspace | **Mission Control** | one active mission's live LEAD/specialist graph, click-to-drill-in (current step, capability scopes, NT-02 hint), and abort/pause/resume controls | ✅ Live (Ch. Mission Control) |
| Workspace | **Schedules** | cron routines: config/operator/agent-created schedules, with create/toggle/delete + the agent-proposal approval flow | ✅ Live (Ch. Chime) |
| Knowledge | **Memory** | self-learning memory browser: topics + entries + search (T) **+ knowledge graph** — see §13 | ✅ Live (Ch. T + MG) |
| Knowledge | **Wiki** | knowledge-wiki browser: synthesized per-topic pages (LLM summary + co-occurrence backlinks + source-entry count) over read-only IPC | ✅ Live (Ch. Codex) |
| Knowledge | **Graph** | typed knowledge-graph view: entity nodes + directed, predicate-labeled relation edges (force-laid-out) over read-only IPC; distinct from the Memory co-occurrence graph | ✅ Live (Ch. Lattice) |
| Agent | **Create**\* | the guided agent-creation flow (Profile → Persona seed → access); first-run lands here when the Profile isn't yet declared | ✅ Live (Ch. Genesis) |
| Agent | **Agents** | persona / soul / profile editor: direct Profile write + persona-governance loop (proposals + revert) — see §9 | ✅ Live (Ch. V) |
| Agent | **Skills** | the skill library: every skill (operator-taught / agent-authored / agent-refined) with its WH.2 effectiveness, provenance, `domain`, version, lineage + the procedure on demand; pending proposals link to Agents; read-only `GetSkills` IPC | ✅ Live (Ch. Repertoire) |
| Agent | **Teams** | the Nonagon roster: team header + member cards (role / trust / scopes / tools / soul) — see §10 | ✅ Live (Ch. Y) |
| System | **Documents** | file browser + **editor** over the agent workspace + the access-scoped fs_root — see §11, §14 | ✅ Live (Ch. Z + DW) |
| System | **Audit** | the dedicated, paginated Audit screen — reuses the Command Center's `AuditFeed` row-renderer rather than a second copy of the same markup | ✅ Live (`/classic` retirement) |
| System | **Sessions** | every active daemon session (channel, trust tier, created/last-active), replacing `/classic`'s own sessions pane | ✅ Live (`/classic` retirement) |
| System | **Gallery** | recent images generated via the configured `comfyui` `[[mcp_server]]`, read from ComfyUI's own `/history` API | ✅ Live (Studio Gallery) |
| System | **Notifications** | configured notify targets (read-only) + dispatch history, for missions/schedules that notify outside the Studio | ✅ Live (Ch. Herald) |
| System | **Loop** | the daemon's autonomous loop control: start/stop/status/log/skip, backlog add/list — full read+write IPC (`QueryPayload::Loop*`) | ✅ Live |
| System | **Reminders** | pending reminders list, soonest-first — read-only `GetReminders` IPC, shared with the TUI Dashboard | ✅ Live |
| System | **MCP** | each configured MCP server's last-start health (connected + tool count, or failed + reason) | ✅ Live (Ch. Lantern) |
| System | **Tools** | a read-only, searchable catalog of every registered tool (name, capability base, minimum trust tier, description) | ✅ Live (Ch. Almanac) |
| System | **Voice** | `[voice]` config editor + readiness check + launch command (audio runs host-side) — see §12 | ✅ Live (Ch. Voice) |
| System | **Settings** | the first config **write** surface: access level + autonomy level (both confirm-first) + budgets editable; provider/model read-only — see §8 | ✅ Live (Ch. U, + Reins) |
| System | **Guide** | the in-app end-user guide — the `docs/guide/*.md` pages rendered in the Studio (see `guide.rs`); pure static content, no daemon IPC | ✅ Live |

\* **Create** only appears pre-genesis — before an agent Profile is
declared. Once a Profile exists, the entry is hidden (`Sidebar`'s
`genesis_done` conditional in `main.rs`); editing then lives in
Agents/Settings instead.

The reference mockups for the locked look: `aivyx-brand/assets/stitch/`
`aivyx_command_center`, `aivyx_missions_orchestration`, `the_terminal`.
```

- [ ] **Step 3: Verify the Group column and row order match the real `Sidebar` code**

Run:

```bash
grep -n "vec!\[" -A 20 crates/aivyx-web/src/main.rs | sed -n '/fn Sidebar/,/^fn /p'
```

If that doesn't isolate it cleanly, instead read `crates/aivyx-web/src/main.rs` at the `fn Sidebar` definition directly (search for `fn Sidebar(view: Signal<View>`) and read through the end of its `groups: Vec<NavGroup> = vec![...]` literal, plus the `agent_group` literal above it.

Confirm, item by item:
- The blank/unlabeled group contains only `Command`.
- The `"Workspace"` group contains, in order: `Chat`, `Missions`, `Mission Control`, `Schedules`.
- The `"Knowledge"` group contains, in order: `Memory`, `Wiki`, `Graph`.
- The `"Agent"` group contains, in order (when a Profile exists — the common case, matching the table's footnote): `Agents`, `Skills`, `Teams`; when it doesn't yet exist, `Create` is prepended before `Agents`. The table lists `Create` first in this group with a footnote, which correctly represents the pre-genesis ordering.
- The `"System"` group contains, in order: `Documents`, `Audit`, `Sessions`, `Gallery`, `Notifications`, `Loop`, `Reminders`, `MCP`, `Tools`, `Voice`, `Settings`, `Guide`.

If any of the above doesn't match the new table exactly, fix the table (not the code — this task only touches documentation) before proceeding.

- [ ] **Step 4: Spot-check 4 "Maps to" cells against their cited source**

Run each of these and confirm the quoted phrase (or a clear paraphrase of it, not a contradiction) appears in the output:

```bash
grep -n "Chapter Chime" crates/aivyx-web/src/main.rs
```
Expected: a doc comment on the `Schedules` variant mentioning "cron routines" and "agent-proposal approval flow" — matches the new table's Schedules row.

```bash
grep -n "Chapter Lantern" crates/aivyx-web/src/main.rs
```
Expected: a doc comment on the `Mcp` variant mentioning "last-start health" and "connected + tool count" — matches the new table's MCP row.

```bash
grep -n "Chapter Genesis" crates/aivyx-web/src/main.rs
```
Expected: a doc comment on the `Onboarding` variant mentioning "guided agent-creation flow" and "Profile → Persona seed → access" — matches the new table's Create row.

```bash
grep -n "AuditFeed row-renderer" crates/aivyx-web/src/main.rs
```
Expected: the doc comment immediately above `fn AuditPanel` — matches the new table's Audit row.

- [ ] **Step 5: Confirm Voice actually shipped (the one State-column change to an existing row)**

Run:

```bash
grep -n "fn VoicePanel" crates/aivyx-web/src/main.rs
grep -n "View::Voice => rsx" crates/aivyx-web/src/main.rs
```

Expected: both produce a match (confirms `VoicePanel` exists and is wired to `View::Voice`), justifying the State-column change from "🔨 In progress" to "✅ Live" in the new table.

- [ ] **Step 6: Confirm the markdown table itself is well-formed**

Run:

```bash
awk 'NR>=85 && NR<=115 && /^\|/ {print NF, $0}' docs/FRONTEND.md
```

Every printed line should show the same field count (each row has the same number of `|`-delimited columns as the header row: `Group`, `Nav item`, `Maps to`, `State` — 4 columns, so `awk`'s `NF` split on the default separator will vary since cells contain spaces, so instead just visually confirm each row starts and ends with `|` and has exactly 3 more internal `|` separators, i.e. 4 `|` characters total per row before the trailing one — the simplest direct check is:

```bash
sed -n '85,111p' docs/FRONTEND.md | grep -o '|' | wc -l
```

This counts total `|` characters across the header separator row + 23 data rows (25 lines × 5 `|` each = 125) — if the count is significantly different, a row is malformed; open the file and inspect visually.

- [ ] **Step 7: Commit**

```bash
git add docs/FRONTEND.md
git commit -s -m "docs: reconcile FRONTEND.md's Studio screen inventory with real code

The table listed 13 of the real 23 sidebar screens. Adds the 11 missing
rows (Schedules, Notifications, Loop, Reminders, MCP, Tools, Guide,
Create/Onboarding, Gallery, Audit, Sessions), each grounded directly
against its own View enum doc comment or nearest existing source.
Also corrects Voice's stale 'In progress' state to 'Live' (VoicePanel
and its IPC are real and wired in). Adds a Group column mirroring the
real Sidebar component's own grouping so future drift is easier to
spot. See docs/superpowers/specs/2026-09-07-frontend-screen-inventory-reconciliation-design.md."
```

---

## Self-Review

**1. Spec coverage:** The spec's exact replacement table (all 23 rows + footnote) is reproduced verbatim in Step 2. The spec's "Testing" section's 3 verification points map onto Steps 3 (group/order diff), 4 (source spot-checks), and 6 (markdown well-formedness) — Step 5 adds one more concrete check (Voice's shipped status) that the spec's Context section asserted but the Testing section didn't explicitly list a check for. No gaps.

**2. Placeholder scan:** No TBD/TODO. Step 1's "if the file has drifted... stop and report" is a legitimate escape hatch for a precondition check, not a placeholder for missing content — the expected content itself is given in full.

**3. Type consistency:** N/A — no code, functions, or types are introduced by this plan.

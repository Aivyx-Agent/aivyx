# Reconciling FRONTEND.md's Studio Screen Inventory

**Status: approved, ready for implementation planning.**

## Context

`docs/FRONTEND.md`'s §3 "Studio screen inventory" table has been flagged
stale in three consecutive phase retrospectives (188, 189, 190) and never
picked up. Direct comparison against the real code confirms the staleness
is real and larger than any retrospective specified:

- The real `View` enum (`crates/aivyx-web/src/main.rs`) has 24 variants;
  the real `Sidebar` component renders 23 of them as nav items (all but
  `Onboarding`, which only appears pre-genesis — before an agent Profile
  is declared — per its own doc comment and the `Sidebar` function's
  `genesis_done` conditional).
- The table currently lists only 13 rows. **11 real screens are missing
  entirely**: Schedules, Notifications, Loop, Reminders, MCP, Tools,
  Guide, Onboarding (the conditional "Create" nav item), Gallery, Audit,
  Sessions.
- **One existing row is itself stale**: Voice is marked "🔨 In progress,"
  but `VoicePanel`, the `GetVoiceSettings`/`SetVoice` IPC, and the full
  `Voice.0`–`Voice.4` phase plan documented in this same file's own §12
  are all real and wired into `View::Voice` (confirmed directly:
  `VoicePanel` exists at `main.rs:6699`, `View::Voice => rsx! {
  VoicePanel {} }` at `main.rs:1246`). It shipped; the state marker never
  got updated.

This is a documentation-only fix — no code changes, no live-backend
infrastructure needed (this was the item scoped down from an earlier,
now-paused attempt to scope Chapter I's separate "polish" placeholder,
which needs live-backend infra this environment doesn't currently have
running; this table fix needs none).

## Approach

Replace §3's single flat 13-row table with one table covering all 23
real nav items (plus a footnote for the conditional 24th), adding a
**Group** column that mirrors the real `Sidebar` component's own 5
groups and item order exactly (blank/Command, Workspace, Knowledge,
Agent, System) — rather than just appending the 11 missing rows to the
existing flat list. Grouping the table the same way the code groups the
sidebar means a future nav-item addition that isn't mirrored in the doc
looks obviously incomplete (a group with a missing member), rather than
silently absent from an undifferentiated list — the same failure mode
that let this drift for 3 phases.

Every new row's "Maps to" text is copied directly from that screen's own
`View` enum doc comment in `main.rs` where one exists, or (for `Loop`,
`Reminders`, `Audit`, which have no enum-level doc comment) from the
nearest grounded existing prose: `Audit`'s `AuditPanel` doc comment,
and `Loop`/`Reminders`'s own established descriptions already written in
`docs/ROADMAP.md`'s Phase 186/188 entries. Nothing is invented — every
description traces to an existing comment or doc line.

The 13 existing rows' "Maps to" text and chapter references are
unchanged, except Voice's **State** column, which changes from
"🔨 In progress" to "✅ Live" (its "Maps to" text is untouched — it was
already accurate).

### The exact replacement table

Full replacement for `docs/FRONTEND.md`'s current §3 table (everything
between the `## 3. Studio screen inventory...` heading and the
`The reference mockups...` line that follows it):

```markdown
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
```

## Testing

Documentation-only change — no automated tests apply. Verification is
grounding each row directly against the real code rather than trusting
this spec's own transcription:

1. Diff the new table's **Nav item** column (23 rows + the Create
   footnote) against `Sidebar`'s real `groups`/`agent_group` literals in
   `crates/aivyx-web/src/main.rs` — same items, same grouping, same
   order.
2. For every row whose "Maps to" text changed or is new, grep the cited
   source (a `View` enum doc comment, `AuditPanel`'s doc comment, or the
   named `docs/ROADMAP.md` entry) and confirm the table's wording doesn't
   contradict it.
3. Render `docs/FRONTEND.md` (or just re-read the diff) to confirm the
   markdown table itself is well-formed (correct column count on every
   row, no broken pipes).

## Self-review

- **Placeholder scan:** none — the full replacement table is given
  verbatim, every cell filled.
- **Internal consistency:** the Group column's order matches the real
  `Sidebar` code's group order exactly (—, Workspace, Knowledge, Agent,
  System), and within each group the row order matches that group's
  real item order.
- **Scope check:** single file, single section, no code changes — small
  enough for one implementation task.
- **Ambiguity check:** the "pull from the nearest grounded source, don't
  invent" rule is applied concretely for every new/changed row above,
  not left as a general instruction for the plan to interpret.

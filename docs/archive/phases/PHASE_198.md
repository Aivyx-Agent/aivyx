# Phase 198 — Chapter Picket Finding 3: Close the Productivity-Integration Coverage Gap

**Chapter Picket follow-up — [SHIPPED] 2026-09-06.**

## Goal (carried from PHASE_196.md's deferred Finding 3)

PHASE_196.md logged Finding 3 as deliberately deferred: the injection
marker list was tuned for a coding agent's file/web content and had
never been evaluated against Gmail/Calendar/Slack/MCP traffic, framed
as a false-positive/calibration risk on content *assumed* to already
be scanned. Phase 198 was scoped to finally pick this up.

## What grounding found — and why it reframed the whole phase

Direct grep across every `impl Tool for` site in `aivyx-gmail`,
`aivyx-calendar`, `aivyx-drive`, `aivyx-contacts`, `aivyx-notion`,
`aivyx-obsidian`, `aivyx-n8n`, and `aivyx-toolkit` found **zero**
overrides of `Tool::output_is_untrusted()` (default `false`) anywhere
in those eight crates. The original framing was wrong about coverage:
this content was neither fenced by Bulwark nor scanned by Picket at
all — a real, open prompt-injection surface (hidden instructions in an
email body or calendar description an agent later reads and may act
on autonomously), not a calibration problem on content already being
scanned. Third-party MCP servers and generic tool-process integrations
(Kitchen, Applications) were already covered; only the first-party
productivity integrations were not. A ninth gap of the same class
turned up alongside them: `aivyx-toolkit`'s `WebSearch`, a genuine
HTTP-backed external search tool with the same missing override.

The user chose to close the coverage gap in this phase, explicitly
deferring the other half of Finding 3's original framing (a config
knob to disable/tune the tripwire) as a separate, still-open
follow-up.

## What shipped

- **28 tools across 9 crates now flag their output as untrusted**
  (`fn output_is_untrusted(&self) -> bool { true }`), classified by
  one principle: a tool's output is untrusted if it is
  externally-authored content the operator didn't just supply via the
  same call (an email body, a calendar event, a Drive file, a search
  result); left untouched if it's an internally-generated confirmation
  of an action the agent/user itself initiated (`GmailSend`,
  `CalendarCreateEvent`, `NotionCreatePage`, and every other
  send/create/update/delete-shaped tool across all 9 crates). Confirmed
  the single, centralized call site (`Agent::run_tool_call`,
  `crates/aivyx-core/src/agent.rs:1410`) already applies both Bulwark's
  fencing and Picket's active scan uniformly to any tool returning
  `true` here — zero turn-loop changes needed anywhere, one mechanical
  trait-method addition per tool.
- **`docs/THREAT_MODEL.md` §5.3 corrected** — two claims that were
  stale independent of this phase's own work: its coverage list never
  named the productivity integrations even before this phase, and its
  claim that "Aivyx still has no content-level scanner for
  prompt-injection payloads" was false since Chapter Picket shipped
  `aivyx-injection-guard` in Phases 194-196 and this document was never
  updated to say so.
- **28 dedicated unit tests**, one per newly-flagged tool, matching the
  existing precedent (`fs_read_output_is_untrusted_for_bulwark`) — 10
  of the 28 files had no existing tool-construction test at all and got
  a new `make_tool()` helper; the other 18 reused an existing one.
- **The final whole-branch review (Opus) caught a real, if narrow,
  defect class no task-level review could have**: five of the 28
  override comments — each written to explain *why* that specific
  tool's content is externally authored — described content the tool's
  own `execute()` body doesn't actually return, verified by reading
  each tool's real projection logic. `gmail.search`'s comment claimed
  message snippets/subjects; the tool actually returns only
  `{id, thread_id}` pairs. `n8n.list_executions`'s comment claimed
  externally-authored execution results; the tool deliberately projects
  those away (and has its own pre-existing test asserting so). Three
  more (`contacts.get`, `drive.search`, `n8n.list_workflows`) named
  fields (notes, snippets, full definitions) those tools don't return.
  The `true` flag itself was correct in every case — either genuine
  externally-authored text is still returned, or the flag is deliberate
  defense-in-depth against a future widening of the projection — only
  the audit-trail comment text was wrong. All five corrected in one
  fix commit.
- **The same review adjudicated a disputed "Critical" finding from
  Task 8's own task-level review**: a new test in `aivyx-toolkit`'s
  `web_search.rs` landed between two `use` statements inside `mod
  tests`, which that reviewer flagged as violating "exact before/after
  text" matching. Verified directly: this is valid, compiling Rust (no
  declaration-order requirement within a module), the exact same
  pattern the plan used successfully across every task on this branch,
  and — checked independently by the final reviewer — genuinely recurs
  in two more files (`aivyx-obsidian`'s `get_note.rs` and `search.rs`).
  Downgraded from Critical to a non-blocking Minor style note; left
  as-is in all three files, not fixed.

## The result

Every real productivity-integration content read the agent can trigger
autonomously — email, calendar, Drive, contacts, Notion, Obsidian, n8n,
and toolkit web search — now gets the same Bulwark-fencing +
Picket-scanning protection `web.fetch`/`fs.read`/MCP output already
had. Chapter Picket's Finding 3 coverage half is closed; 28 tools, 0
turn-loop changes, 0 copy-paste test errors (independently verified by
the final review against every `make_tool()`'s return type).

## Known follow-ups (not done here, logged for whenever they matter)

- **The config-knob half of Finding 3 remains open**: no operator-facing
  way to disable or tune the tripwire, independent of what content gets
  scanned. Explicitly deferred to a separate phase per the user's own
  scoping decision at the start of this phase.
- **The marker list itself is still ported verbatim, not expanded** —
  unchanged from the follow-up already logged in `docs/ROADMAP.md`'s
  Chapter Picket section. Real-usage-driven additions to
  `INJECTION_MARKERS` (e.g. phrasings specific to email/calendar social-
  engineering rather than coding-agent contexts) are a candidate for
  whenever real operation surfaces a miss.
- **`docs/THREAT_MODEL.md`'s residual staleness one paragraph past this
  phase's fix**: the sentence immediately after the corrected claim
  ("Beyond Bulwark's fencing, the remaining defense is capability
  gating") now silently omits Picket's scanner as an intermediate layer.
  Correctly left untouched here — the plan scoped Task 9 to exactly two
  claims — but worth a follow-up pass.
- **The interleaved-`use`-statement style** in `web_search.rs`,
  `obsidian/get_note.rs`, and `obsidian/search.rs` (a new test landing
  between two pre-existing `use` lines rather than after all of them) —
  confirmed harmless and non-blocking by the final review, left as-is
  in all three files. Purely cosmetic if anyone wants to tidy it later.

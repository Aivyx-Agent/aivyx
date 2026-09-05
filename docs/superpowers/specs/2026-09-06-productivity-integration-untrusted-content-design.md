# Close the Productivity-Integration Untrusted-Content Coverage Gap

**Status: approved, ready for implementation planning.**

## Context

Chapter Picket (Phases 194-196) added `aivyx-injection-guard`, an active
prompt-injection tripwire, layered on top of Chapter Bulwark's existing
passive fencing (`fence_untrusted_output`). Both mechanisms are driven by
one trait method, `Tool::output_is_untrusted()` (default `false`), which
`Agent::run_tool_call` (`crates/aivyx-core/src/agent.rs:1410`) checks at a
single, centralized call site shared by every tool in the system — no
per-tool-type special-casing anywhere else.

PHASE_196.md logged a follow-up ("Finding 3, deliberately deferred") that
the injection marker list was tuned for a coding agent's file/web content
and had never been evaluated against Gmail/Calendar/Slack/MCP traffic,
where phrases like "you are now subscribed" are routine — framed as a
false-positive/calibration risk on content that was *assumed* to already
be covered.

Grounding done this session found that assumption was wrong: `grep` across
every `impl Tool for` site in `aivyx-gmail`, `aivyx-calendar`,
`aivyx-drive`, `aivyx-contacts`, `aivyx-notion`, `aivyx-obsidian`,
`aivyx-n8n`, and `aivyx-toolkit` shows **zero overrides** of
`output_is_untrusted()` anywhere in those eight crates. Third-party MCP
servers (`aivyx-mcp`'s three proxy types) and generic tool-process
integrations (`aivyx-tool::ToolProxy`, covering Kitchen/Applications/any
operator-configured `[[tool_process]]` entry) are already covered; the
first-party productivity integrations are not. This is a real, currently
open prompt-injection surface — a well-documented real-world attack class
(hidden instructions in email/calendar/document content an agent later
reads and may act on autonomously) — not merely a tuning problem on
already-scanned content.

## Scope

Three parts, one phase:

1. Flag 28 tools' output as untrusted across 9 crates (the 8 productivity
   integrations plus `aivyx-toolkit`'s `WebSearch`, a ninth real gap found
   during this session's grounding — a genuine HTTP-backed external search
   tool with the same missing override).
2. Add a unit test per newly-flagged tool confirming the override.
3. Correct `docs/THREAT_MODEL.md` §5.3, which is stale in two ways
   independent of this phase's own changes: it still lists Bulwark's
   coverage as only "web fetches, file reads, MCP tool output, operator-
   provided context files" (missing productivity integrations even before
   this phase, and needing updating after), and it still claims "Aivyx
   still has no content-level scanner for prompt-injection payloads... not
   detecting and blocking malicious patterns" — false since Chapter
   Picket shipped exactly that scanner in Phases 194-196 and this
   correction was never made.

Explicitly out of scope, per the user's own scoping decision: a
config knob to disable/tune the tripwire (the other half of the original
Finding 3 framing) is a separate, still-open follow-up, not part of this
phase.

## 1. Tool classification

**Principle:** a tool's output is untrusted if it is externally-authored
content the operator did not just author via the same call (an email
body, a calendar event's description, a Drive file's contents, a
Notion page, an Obsidian note, an n8n execution result, a web search
result). A tool's output is *not* flagged if it is an internally-generated
confirmation of an action the agent/user itself just initiated (a send
receipt, a create/update/delete confirmation) — the content that made it
untrusted-worthy, if any, was supplied by the agent's own call arguments,
not read back from an external source.

**28 tools get `fn output_is_untrusted(&self) -> bool { true }` added to
their existing `impl Tool for ...` block:**

| Crate | Tool | File:line of `impl Tool for` |
|---|---|---|
| aivyx-gmail | `GmailSearch` | `crates/aivyx-gmail/src/tools/search.rs:56` |
| aivyx-gmail | `GmailRead` | `crates/aivyx-gmail/src/tools/read.rs:68` |
| aivyx-calendar | `CalendarGetEvent` | `crates/aivyx-calendar/src/tools/get_event.rs:40` |
| aivyx-calendar | `CalendarListEvents` | `crates/aivyx-calendar/src/tools/list_events.rs:74` |
| aivyx-calendar | `CalendarUpcoming` | `crates/aivyx-calendar/src/tools/upcoming.rs:78` |
| aivyx-calendar | `CalendarListCalendars` | `crates/aivyx-calendar/src/tools/list_calendars.rs:61` |
| aivyx-drive | `DriveListDrives` | `crates/aivyx-drive/src/tools/list_drives.rs:69` |
| aivyx-drive | `DriveListFolder` | `crates/aivyx-drive/src/tools/list_folder.rs:53` |
| aivyx-drive | `DriveSearch` | `crates/aivyx-drive/src/tools/search.rs:64` |
| aivyx-drive | `DriveGetMetadata` | `crates/aivyx-drive/src/tools/get_metadata.rs:41` |
| aivyx-drive | `DriveDownloadFile` | `crates/aivyx-drive/src/tools/download_file.rs:66` |
| aivyx-drive | `DriveRecentChanges` | `crates/aivyx-drive/src/tools/recent_changes.rs:72` |
| aivyx-drive | `DriveRecentFiles` | `crates/aivyx-drive/src/tools/recent_files.rs:72` |
| aivyx-drive | `DriveRecentActivity` | `crates/aivyx-drive/src/tools/recent_activity.rs:72` |
| aivyx-contacts | `ContactsGet` | `crates/aivyx-contacts/src/tools/get.rs:40` |
| aivyx-contacts | `ContactsSearch` | `crates/aivyx-contacts/src/tools/search.rs:43` |
| aivyx-contacts | `ContactsList` | `crates/aivyx-contacts/src/tools/list.rs:42` |
| aivyx-notion | `NotionSearch` | `crates/aivyx-notion/src/tools/search.rs:53` |
| aivyx-notion | `NotionGetPage` | `crates/aivyx-notion/src/tools/get_page.rs:54` |
| aivyx-notion | `NotionListDatabase` | `crates/aivyx-notion/src/tools/list_database.rs:50` |
| aivyx-obsidian | `ObsidianGetNote` | `crates/aivyx-obsidian/src/tools/get_note.rs:32` |
| aivyx-obsidian | `ObsidianSearch` | `crates/aivyx-obsidian/src/tools/search.rs:49` |
| aivyx-obsidian | `ObsidianListFolder` | `crates/aivyx-obsidian/src/tools/list_folder.rs:34` |
| aivyx-n8n | `N8nGetWorkflow` | `crates/aivyx-n8n/src/tools/get_workflow.rs:55` |
| aivyx-n8n | `N8nListWorkflows` | `crates/aivyx-n8n/src/tools/list_workflows.rs:52` |
| aivyx-n8n | `N8nGetExecution` | `crates/aivyx-n8n/src/tools/get_execution.rs:46` |
| aivyx-n8n | `N8nListExecutions` | `crates/aivyx-n8n/src/tools/list_executions.rs:52` |
| aivyx-toolkit | `WebSearch` | `crates/aivyx-toolkit/src/tools/web_search.rs:77` |

**Left at the trait's default `false` (internally-generated confirmation
outputs) — not touched by this phase:** `GmailSend`, `GmailDraft`,
`CalendarCreateEvent`/`UpdateEvent`/`DeleteEvent`, `DriveCreateFolder`/
`UploadFile`/`DeleteFile`, `ContactsCreate`/`Update`/`Delete`,
`NotionCreatePage`/`UpdatePageProperties`/`AppendBlocks`/`ArchivePage`,
`ObsidianCreateNote`/`UpdateNote`/`DeleteNote`, `N8nCreateWorkflow`/
`UpdateWorkflow`/`ActivateWorkflow`/`DeactivateWorkflow`/`DeleteWorkflow`/
`ExecuteWorkflow`, and every `aivyx-toolkit` tool other than `WebSearch`
(`ConvertUnits`, `ConvertTime`, `CalcEval`, `DateDiff`, `DateAdd`,
`HealthCheck*`, `Task*`, `Budget*`).

## 2. Implementation

Each of the 28 tools gets exactly one addition to its existing `impl Tool
for X` block, matching the codebase's own established idiom exactly (e.g.
`crates/aivyx-core/src/tools/fs.rs:236-237`):

```rust
    // Chapter Picket follow-up (Finding 3) — <tool-specific one-line reason
    // this content is externally authored, not agent/operator-generated>.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

No changes anywhere in `crates/aivyx-core/src/agent.rs` or any other
turn-loop code — confirmed the single call site at `agent.rs:1410` already
applies both Bulwark's fencing and Picket's active scan uniformly to any
tool returning `true` here, with zero tool-type-specific gating elsewhere
in the codebase.

## 3. Testing

One unit test per newly-flagged tool, matching the existing precedent
(`fs_read_output_is_untrusted_for_bulwark`, `crates/aivyx-core/src/tools/fs.rs:1989`):
construct the tool, assert `output_is_untrusted()` returns `true`. Grouped
into each crate's existing test file/module (not one new file per tool) —
the exact grouping is a plan-level decision based on each crate's current
test-file layout, not a spec-level one.

## 4. Documentation fix

`docs/THREAT_MODEL.md` §5.3 ("Prompt injection beyond capability gating")
needs two corrections, both real and both predating this phase:

- Its coverage list ("web fetches, file reads, MCP tool output, operator-
  provided context files") never mentioned the productivity integrations
  even before this phase (they simply weren't covered) — after this
  phase ships, it should read accurately: web fetches, file reads, MCP
  tool output, operator-provided context files, **and the productivity
  integrations' externally-authored content (Gmail, Calendar, Drive,
  Contacts, Notion, Obsidian, n8n) plus toolkit's web search**.
- Its claim *"Aivyx still has no content-level scanner for prompt-
  injection payloads the way Hermes Agent's Tirith does; Bulwark's
  fencing works by changing how untrusted content is presented to the
  model, not by detecting and blocking malicious patterns within it"* is
  false and has been since Chapter Picket shipped (Phases 194-196,
  2026-09-05) — `aivyx-injection-guard` is exactly that content-level
  scanner. This sentence must be corrected to describe the real,
  current state: Bulwark's structural fencing plus Picket's active
  phrase-list scan, which escalates the turn on a match.

The document's own "last reviewed" line (mentioned in the section's
existing self-correcting comment, "still reads 'Phase 180 exit'") is
out of scope for this phase to hunt down and fix generally — only the
two factual claims above, which this phase's own work directly
contradicts, need correcting here.

## Self-review

- **Placeholder scan:** none — the full 28-tool table gives exact
  file:line for every change; the trusted/untrusted classification for
  every tool in all 9 crates is enumerated, not summarized.
- **Internal consistency:** Section 1's classification, Section 2's
  mechanical implementation, and Section 4's doc fix all agree on what
  "coverage" means and which tools it now includes.
- **Scope check:** one phase, three well-bounded parts (code + tests,
  doc fix), no config-knob work (explicitly deferred per the user's own
  scoping decision) — sized similarly to Chapter Picket's own phases.
- **Ambiguity check:** the read/write classification principle was
  confirmed with the user before this document was written; every tool
  across all 9 crates is explicitly listed on one side (the 28-row
  table) or the other (the "left at default `false`" list), leaving
  none unclassified. (Corrected during self-review: an earlier draft
  of this document mis-added the table to 27; the table itself was
  always the source of truth and is unchanged — only the prose count
  was wrong.)

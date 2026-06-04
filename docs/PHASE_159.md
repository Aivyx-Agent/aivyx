# Phase 159 — Drive Activity API: `drive.recent_activity`

**New tool, not a close-out.** Phase 145 shipped
`drive.recent_files` + `drive.recent_changes`
backed by Drive API v3's `/files` endpoint with
`modifiedTime > '<RFC3339>'` filters. Those tools
answer "what files moved" — but not "what
*happened* to them." Drive Activity API answers
the second question: who edited, who shared, who
renamed, who commented. Phase 159 wires that
into a new `drive.recent_activity` tool.

## Why this, why now

- **Twice-deferred Recommended.** Phase 156 and
  Phase 157 close-outs both deferred this in
  favor of bundle work; on a third pass the user
  picked it.

- **Phase 145's `recent_*` tools answer "what
  files," not "what activity."** A doc that got
  edited 30 times by 3 people surfaces as one
  entry in `recent_changes`. With
  `drive.recent_activity` the agent can see the
  individual `edit` events with timestamps and
  actors.

- **All substrate already exists.** Reqwest
  client, OAuth refresh, JSON shaping helpers,
  `post_json` shape — Phase 159 extends rather
  than introduces. The only new piece is a
  variant `post_json_activity` that targets the
  Activity API base URL.

- **Operator-facing scope expansion is one-time.**
  Adding `drive.activity.readonly` to
  `DEFAULT_DRIVE_SCOPES` means existing operators
  re-run `aivyx-drive auth init` once. INSTALL.md
  documents the re-auth step.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   the roadmap section + the README row.
   Backfill Phase 158 hash to `dda59c2`.

2. **OAuth scope + API base URL substrate.**
   Three small substrate changes:
   - Add
     `"https://www.googleapis.com/auth/drive.activity.readonly"`
     to `DEFAULT_DRIVE_SCOPES`.
   - Add
     `pub const DRIVE_ACTIVITY_API_BASE:
     &str = "https://driveactivity.googleapis.com/v2"`
     to drive_client.rs.
   - Add `post_json_activity(&self, path:
     &str, body: &Value) -> Result<T,
     DriveClientError>` that mirrors
     `post_json` but uses the activity base
     URL.
   Tests pin the new constant + scope addition.

3. **`drive.recent_activity` tool module.**
   New tool that POSTs to
   `:activity:query` with a body shaped like:
   ```json
   {
     "consolidationStrategy": {"legacy": {}},
     "filter": "time > <unix-ms>",
     "pageSize": <max_results>
   }
   ```
   Response gets shaped into a
   `{activities: [...], count}` envelope where
   each activity has `{timestamp, action_type,
   target_summary, actor_summary}`. Pure-
   substrate response shaper tested with canned
   JSON; live HTTP is operator-validation tier.

4. **Tool registration in harness.** Wire
   `DriveRecentActivity` into:
   - `tools/mod.rs` (`pub mod recent_activity;`
     + `pub use ...`).
   - `main.rs` harness vector (alphabetical
     order, matching the existing list).
   - Tool count regression test bumps from N to
     N+1.

5. **INSTALL + exit + Frozen.** INSTALL.md drive
   section gains a `drive.recent_activity` row.
   The new OAuth scope gets a one-line re-auth
   note in the drive auth section (
   `aivyx-drive auth init` after upgrading).
   Exit doc with prediction-vs-reality. README +
   ROADMAP flip to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** New tool +
  substrate addition; no contract amendment.
  Streak: 49 → **50**.
- **PRODUCT.md** — **Will hold.** Streak:
  49 → **50**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 159 work in `aivyx-drive`. Streak:
  24 → **25**.

## Exit criteria

- [ ] `docs/PHASE_159.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `DEFAULT_DRIVE_SCOPES` extended +
  `DRIVE_ACTIVITY_API_BASE` constant +
  `post_json_activity` helper — Task 2.
- [ ] `drive.recent_activity` tool with input
  schema, parse_input, execute, response shaper
  — Task 3.
- [ ] Tool registered in harness + tool count
  regression — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+18`.

## Honest scope risks at sign-off

- **Re-auth required for existing operators.**
  Adding a new scope to
  `DEFAULT_DRIVE_SCOPES` means existing tokens
  don't cover the new scope. The first
  `drive.recent_activity` call returns a 403
  with the Google "insufficient permissions"
  message; operators have to re-run
  `aivyx-drive auth init`. INSTALL.md documents
  this explicitly.

- **Activity consolidation default = `legacy`.**
  The API has three strategies: `legacy`,
  `none`, `consolidated`. Phase 159 hardcodes
  `legacy` (matches the Drive UI's "activity
  feed" semantics). Operators who want raw
  un-consolidated events have no knob; Phase
  160+ candidate.

- **No actor / target filters.** The Activity
  API supports filtering on actor (e.g.
  `actor.user.knownUser.isCurrentUser = true`)
  and target (e.g. specific item / parent
  folder). Phase 159 ships time-window-only
  filtering. Phase 160+ candidate.

- **Response shape is best-effort agent-
  ergonomic.** The raw Activity object has a
  deeply nested actions / actors / targets
  structure. Phase 159 picks "first action,
  first actor, first target" as the surface
  shape; multi-action / multi-target activities
  collapse to their first-element representation.
  Honest substrate-loss for ergonomics.

- **`drive.recent_activity` doesn't compose with
  `parent_folder_id` or `drive_id` scope.** The
  Activity API has its own filter DSL that
  doesn't map 1:1 with the Drive v3 query
  shapes. Phase 159 ships time-window-only;
  scope composition is a Phase 160+ task.

- **Forty-eighth consecutive deferral of the
  Channel Activation Milestone.** Per operator
  framing — intentional hold.

## Direction after Phase 159

After Phase 159, the Drive surface has a
genuine "what happened" tool. Phase 160+
candidates:

1. **drive.recent_activity actor / target
   filters.**
2. **drive.recent_activity consolidation knob
   (none / consolidated).**
3. **drive.recent_activity + parent_folder_id
   composition.**
4. **max_concurrent throttle on
   walk_folder_tree.** (Phase 157 honest-debt.)
5. **Operator-tunable image size cap.**
6. **URL fetch timeout.**
7. **HEAD pre-fetch for size check.**
8. **Authenticated URL fetch.**
9. **PDF / SVG / TIFF media type support.**
10. **Mid-recording or mid-reply /image
    command.**
11. **Clipboard-based image source.**
12. **Voice abort UX knob.**
13. **Silero ONNX VAD.**
14. **Streaming ASR.**
15. **Wake-word activation.**
16. **macOS streaming variant.**
17. **Lock-free AudioIn detector.**
18. **calendarList cache TTL knob.**
19. **access_role deprecation.**
20. **Budget category migration tool.**
21. **Budget currency / rust_decimal.**
22. **Multi-category trend breakdown.**
23. **Trend smoothing / moving average.**
24. **Bulk budget operations.**
25. **Proactive reminder dispatch.**
26. **Relative-time localization.**
27. **whisper-cpp-plus rehabilitation.**
28. **`build_agent_stack` substrate-tier
    promotion.**
29. **Channel Activation Milestone** —
    still held intentionally; 48th
    consecutive deferral at Phase 159
    open.

## Prediction vs reality

_Populated at Phase 159 exit. Predictions at
sign-off: DESIGN.md HOLD → 50; PRODUCT.md HOLD
→ 50; lib.rs HOLD → 25; zero new deps; test
count delta `+10` to `+18`; zero clippy
warnings._

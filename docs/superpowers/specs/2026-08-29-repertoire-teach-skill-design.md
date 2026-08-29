# Studio "Teach a skill" form (V09_PLAN row 6 / POLISH_WAVES.md sub-project 3) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 3 ("Repertoire / governed-write completions") lists three items: a Studio "Add skill" write UI, Repertoire approve-in-place, and invocation history in the Repertoire screen. Before designing anything, the real current state of `docs/REPERTOIRE.md` (Chapter Repertoire's own locked reference) and the actual shipped code were checked directly — not assumed from the plan's own description, matching this workspace's now-established discipline after finding the same kind of staleness in `V09_PLAN.md`'s row 3/5 and `POLISH_WAVES.md`'s own sub-project 2 scoping.

## Research findings (grounded in the real, current code)

**Two of the three listed items are already shipped.** `docs/REPERTOIRE.md`'s own "Out" section marks both "approve / edit / reject in the Skills screen" and "surfacing skill invocation history" as done, tagged "pre-v0.4.0" (i.e., shipped long before this v0.9 phase, under Chapter Repertoire's own RP.2/RP.3 phases — the plan's row 6 was simply never checked against this doc's later state). Verified directly against `crates/aivyx-web/src/main.rs`, not just the doc's claim:

- **Approve-in-place**: `SkillsPanel` (main.rs:3229) already renders pending skill proposals inline via the existing `ProposalCard` component (main.rs:3283) — real approve/edit/reject, not a pointer to another screen (the doc's own text calling this "a documented deferral" is itself stale; the deferral was later closed).
- **Invocation history**: `SkillCard`-equivalent rendering already shows `"· invoked {view.invocations}×"` per skill (main.rs:3341), a per-skill all-time count from the audit chain.

**The one genuine gap: no Studio "Teach" (add) form.** The backend already fully exists and works: Chapter Tutor's `FrontendMessage::AuthorSkill { id, op: SkillAuthorOp::Teach, name, trigger, procedure }` (`aivyx-ipc/src/protocol.rs:1855`) appends a signed, operator-authored `LearnedSkill` delta to the persona chain via the same `skill_edit` helpers the agent's own `skills.teach` tool uses, then recomputes the shared persona live (adopted next-turn, **no daemon restart required** — this differs from `POLISH_WAVES.md`'s own assumption that this reuses "the proven `toml_edit` writer + ... restart-required UX recipe"; Chapter Tutor's mechanism is a persona-chain append, not a TOML edit, and takes effect immediately). The daemon's own validation (`daemon_server.rs`'s `author_skill_live`, ~line 6463): `validate_skill_name(name)`, rejects a duplicate name for `Teach` ("a skill named {name} already exists — use update"), requires non-empty `trigger`/`procedure`. The CLI (`crates/aivyx-cli/src/bin/aivyx_modules/skills.rs`) already exercises this exact path via `aivyx skills teach <name> <trigger> <procedure>` — proving the backend works; only the Studio-side form was ever missing. The protocol's own doc comment (`protocol.rs:1845`) names "the Studio Skills screen" as an intended caller, confirming this was always meant to be wired here.

**No existing "create new item" form precedent in this file** to mirror exactly — `SchedulesUi` (main.rs:522) has a `confirm_delete` two-step-delete pattern and a `notice: Option<(bool, String)>` mutation-outcome pattern, both reusable, but nothing in this codebase today opens a blank form to create a brand-new object. This design establishes that shape for the first time, kept as simple as the requirement allows.

**The response side**: `DaemonEnvelope::SkillAuthored { id, ok, seq, error }` (`protocol.rs:2203`/`2639`) — `ok: true` with `seq` (the chain seq of the appended delta) on success, `ok: false` with `error` on any validation failure. The ack does **not** echo back the full `LearnedSkill`/`SkillView` — only the CLI needs a "did it work" signal today. The Studio needs the new skill's full data (for its effectiveness bucket display, provenance badge, etc.) to actually show up in the inventory, which the ack alone can't provide.

## Scope

- **In**: a collapsed-by-default "+ Teach a skill" toggle at the top of `SkillsPanel`, opening a 3-field form (name, trigger, procedure) + Submit/Cancel; wiring Submit to the existing `AuthorSkill`/`Teach` message; handling the `SkillAuthored` ack (success: clear + collapse + re-fetch; failure: keep form open with entered values + show the server's own error text).
- **Out**: any change to the backend (`daemon_server.rs`, `aivyx-ipc`) — Chapter Tutor's write path is complete and unmodified by this work. Editing a skill's body in place (stays in chat via `skills.update`, per `REPERTOIRE.md`'s own explicit "Out" list — unchanged by this design). Any new capability base, agent tool, or storage domain — none needed, matching Chapter Repertoire's own "read-only screen, nothing new" governance stance for its read surface; this form uses the human-authority-tier `AuthorSkill` path, which needs no agent scope at all.
- **Correcting `POLISH_WAVES.md`**: this design doc's existence, once approved, should be reflected back into that doc — its sub-project 3 entry currently still lists all three original items and the wrong (`toml_edit`/restart-required) mechanism assumption; both should be corrected once this ships, matching the pattern already established for sub-project 2's own corrections.

## Architecture

### State

New `SkillsUi` struct, mirroring `SchedulesUi`'s existing shape:

```rust
#[derive(Clone, Default, PartialEq)]
struct SkillsUi {
    /// Whether the "Teach a skill" form is expanded.
    form_open: bool,
    name: String,
    trigger: String,
    procedure: String,
    /// `(ok, text)` outcome of the last teach attempt.
    notice: Option<(bool, String)>,
}
```

Registered as a `use_signal(SkillsUi::default)` + `use_context_provider`, same as every other UI-state signal in `App`.

### The form

Toggled by a "+ Teach a skill" button/link above the existing skill-card list. When `form_open`, renders three text inputs bound to `ui.name`/`ui.trigger`/`ui.procedure` (via `oninput` setting the corresponding field) and a Submit button, disabled when any of the three is blank (client-side convenience only — the server remains authoritative). A Cancel button collapses the form without submitting, clearing entered values.

### Submit

```rust
fn author_skill_teach_query(name: String, trigger: String, procedure: String) -> FrontendMessage {
    FrontendMessage::AuthorSkill {
        id: "mc-skill-teach".to_string(),
        op: SkillAuthorOp::Teach,
        name,
        trigger: Some(trigger),
        procedure: Some(procedure),
    }
}
```

Sent on Submit click via `ws.send(...)`.

### Response handling

New arm in the shared response-dispatch match (alongside the existing `SkillForgotten` arm):

```rust
DaemonEnvelope::SkillAuthored { ok, error, .. } => {
    if ok {
        skills_ui.write().notice = Some((true, "Skill taught.".into()));
        skills_ui.write().form_open = false;
        skills_ui.write().name.clear();
        skills_ui.write().trigger.clear();
        skills_ui.write().procedure.clear();
        ws.send(skills_query()); // re-fetch to pick up the new skill
    } else {
        let msg = error.unwrap_or_else(|| "Teach failed.".into());
        skills_ui.write().notice = Some((false, msg));
        // form stays open with the operator's entered values intact
    }
}
```

`seq` is intentionally unused (matches the design's own scope: no optimistic insert, a re-query is the simplest correct path). `skills_ui` needs threading into the same shared coroutine function(s) (`ws_task`/`read_task`, or wherever this codebase's current equivalent lives after the last chapter's own refactors) the same way `audit_page`/`sessions_page` were threaded in the prior chapter — exact call sites to be re-confirmed fresh during planning, not assumed here (the file has changed shape across several recent chapters; trust a fresh grep over this doc's own memory of line numbers).

## Testing

- No backend changes, so no new backend tests are needed — Chapter Tutor's own existing test coverage (`author_skill_live_teach_update_forget_and_errors` in `daemon_server.rs`) already covers the write path this form calls into.
- Frontend: real `cargo check -p aivyx-web` and `cargo clippy -p aivyx-web --all-targets -- -D warnings` (both confirmed to work natively in this sandbox during the prior chapter) as real verification, plus the real `wasm32-unknown-unknown` build via the rustup toolchain discovered during that same chapter, and a rebuilt+committed `dist/` bundle.
- A live operator walkthrough (typing a real skill name/trigger/procedure, submitting, confirming it appears in the inventory and is usable) remains the one thing this sandbox can't verify — same caveat as every other Studio screen this workspace has built.

## Out of scope

- Any change to Chapter Tutor's backend validation, wire types, or the CLI's own `aivyx skills teach/update/forget` commands.
- Editing an existing skill's trigger/procedure from the Studio (`SkillAuthorOp::Update`) — `REPERTOIRE.md` explicitly keeps this in chat via `skills.update`; not revisited here.
- Any UI for `SkillAuthorOp::Update` at all — only `Teach` is in scope, per the plan's own original ask ("Add skill").
- Correcting `POLISH_WAVES.md`/`V09_PLAN.md`'s own stale text — tracked as a follow-up once this ships, not part of the implementation plan itself (a docs-only change, same pattern as prior chapters).

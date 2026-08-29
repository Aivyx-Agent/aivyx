# Studio "Teach a skill" Form Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Teach a skill" form to the Studio's Skills screen, wired to the already-working `AuthorSkill`/`SkillAuthorOp::Teach` backend the CLI already uses.

**Architecture:** A collapsible form inside the existing `SkillsPanel` component (`crates/aivyx-web/src/main.rs`) — local `use_signal`s for the three text fields and the open/closed toggle (matching the established `SchedulesPanel`'s own "Create schedule" form precedent exactly, not a new pattern), a shared `SkillsUi { notice }` context signal for the submit outcome (matching `SchedulesUi`'s own minimal shape), and one new response-handler arm for `DaemonEnvelope::SkillAuthored` (matching the existing `ScheduleMutated` handler's shape). No backend changes — the wire message, validation, and persistence already exist and are already tested.

**Tech Stack:** Rust, Dioxus (`aivyx-web`).

## Global Constraints

- **Correction from the design doc**: `docs/superpowers/specs/2026-08-29-repertoire-teach-skill-design.md` proposed putting the form's text fields (`name`/`trigger`/`procedure`) and an `open` flag into the shared `SkillsUi` context struct, reasoning that "no existing create-new-item form precedent" existed in this file to copy. That reasoning was wrong — re-checked during planning: `SchedulesPanel`'s own "Create schedule" form (`crates/aivyx-web/src/main.rs`, search for `let mut cron = use_signal(String::new);`) is exactly this shape, and it keeps its text fields as **local** `use_signal`s inside the component, using its shared `SchedulesUi` context signal *only* for `notice` (and `confirm_delete`, unrelated to this feature). This plan follows the real, verified precedent: `SkillsUi` holds only `notice: Option<(bool, String)>`; the three text fields and the open/closed toggle are local signals inside `SkillsPanel`.
- `aivyx-web` compiles and clippy-checks **natively** in this sandbox (no wasm32 target needed) — `cargo check -p aivyx-web` / `cargo clippy -p aivyx-web --all-targets -- -D warnings` both work and are real, required verification steps, not skippable.
- The **real** `wasm32-unknown-unknown` target is also available, via a separate rustup toolchain: `export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"` — use it for the final bundle rebuild step. `dx` (the Dioxus CLI) should already be installed from prior work; if not, `cargo install dioxus-cli --version 0.6.3 --locked` once.
- No backend changes of any kind — `crates/aivyx-channel/src/daemon_server.rs`, `crates/aivyx-ipc/src/protocol.rs`, and the CLI's `crates/aivyx-cli/src/bin/aivyx_modules/skills.rs` are all out of scope and must not be touched.
- Client-side field-blank validation is a UX convenience only; the server (`author_skill_live` in `daemon_server.rs`) remains the authoritative validator (duplicate-name rejection, non-empty trigger/procedure) — do not attempt to replicate its exact rules client-side beyond "not blank."

---

### Task 1: The "Teach a skill" form, submit wiring, and ack handling

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`:
  - Import list (top of file, `use aivyx_ipc::protocol::{...}`) — add `SkillAuthorOp`
  - `SkillsUi` struct — new, placed near `SchedulesUi`'s own definition
  - `App`'s signal declarations (~line 693, alongside `let skills = use_signal(SkillsState::default);`) and `use_context_provider` block (~line 744) — register `skills_ui`
  - `App`'s `use_coroutine` call (~line 726-730) — thread `skills_ui` through
  - `ws_task`'s signature (~line 7290) and its own `spawn(read_task(...))` call (~line 7334-7337) — thread `skills_ui` through
  - `read_task`'s signature (~line 7427) — thread `skills_ui` through (with `mut`, matching every other parameter in this specific signature)
  - `SkillsPanel` component body (~line 3229) — add the form
  - The response-dispatch match inside `read_task` (search for the existing `DaemonEnvelope::SkillForgotten { ok, removed, name, .. } if ok && removed => { ... }` arm, ~line 7712) — add a new `SkillAuthored` arm immediately after it

**Interfaces:**
- Consumes: `aivyx_ipc::protocol::{FrontendMessage, SkillAuthorOp, DaemonEnvelope}` (all pre-existing — `SkillAuthorOp` needs adding to this file's import list, the others are already imported), `SchedulesUi`'s established pattern as the template (not a new abstraction).
- Produces: nothing later work depends on — this is a self-contained, terminal feature.

- [ ] **Step 1: Add `SkillAuthorOp` to the import list**

Find (near the top of `crates/aivyx-web/src/main.rs`):

```rust
use aivyx_ipc::protocol::{
    AuditEntrySummary, DaemonEnvelope, DocEntry, DocFile, EffectivePersonaSummary, FrontendMessage,
    GalleryImage, McpServerStatusView, MemoryEntrySummary, MemoryGraphNode, NotificationHistoryEntry,
    NotifyTargetView, PersonaDeltaSummary,
    PersonaProposalResolution,
    PersonaProposalSummary, PersonaSeedWire, ProfileDraftWire, ProfileSummary, QueryPayload,
    QueryResponsePayload, ScheduleView, SeedSkillWire, SessionSummary, SettingsSnapshot, SkillView,
    StreamEventPayload, ToolCatalogEntry, VoiceSettingsSnapshot,
};
```

Replace with (adding `SkillAuthorOp` alphabetically, right after `SessionSummary`):

```rust
use aivyx_ipc::protocol::{
    AuditEntrySummary, DaemonEnvelope, DocEntry, DocFile, EffectivePersonaSummary, FrontendMessage,
    GalleryImage, McpServerStatusView, MemoryEntrySummary, MemoryGraphNode, NotificationHistoryEntry,
    NotifyTargetView, PersonaDeltaSummary,
    PersonaProposalResolution,
    PersonaProposalSummary, PersonaSeedWire, ProfileDraftWire, ProfileSummary, QueryPayload,
    QueryResponsePayload, ScheduleView, SeedSkillWire, SessionSummary, SettingsSnapshot,
    SkillAuthorOp, SkillView, StreamEventPayload, ToolCatalogEntry, VoiceSettingsSnapshot,
};
```

(If the real current import list has drifted from this exact text by the time you read it — other chapters may have added entries since this plan was written — just add `SkillAuthorOp` to whatever the current list is, in the same `use aivyx_ipc::protocol::{...}` block. Don't fight the diff over unrelated entries.)

- [ ] **Step 2: Add the `SkillsUi` struct**

Find `SchedulesUi`'s definition (search for `struct SchedulesUi`):

```rust
/// Chapter Chime — Schedules screen UI state (the list itself lives in
/// `Dashboard::schedules`, already polled every 5 s).
#[derive(Clone, Default, PartialEq)]
struct SchedulesUi {
    /// `(ok, text)` outcome of the last mutation ack.
    notice: Option<(bool, String)>,
    /// Two-step delete: the schedule_id awaiting its confirm click.
    confirm_delete: Option<String>,
}
```

Immediately after it, add:

```rust
/// Chapter Tutor — Skills screen "Teach a skill" form UI state. The form's
/// own text fields (name/trigger/procedure) and open/closed toggle are
/// local `use_signal`s inside `SkillsPanel` itself, not here — this only
/// holds the cross-cutting mutation-ack outcome, exactly like
/// `SchedulesUi.notice` above (the ack arrives in the shared `read_task`
/// response dispatch, which doesn't have direct access to `SkillsPanel`'s
/// own local component state).
#[derive(Clone, Default, PartialEq)]
struct SkillsUi {
    /// `(ok, text)` outcome of the last `AuthorSkill` (Teach) attempt.
    notice: Option<(bool, String)>,
}
```

- [ ] **Step 3: Register `skills_ui` as a signal + context provider**

Find (~line 693):

```rust
    let skills = use_signal(SkillsState::default);
```

Add immediately after it:

```rust
    let skills_ui = use_signal(SkillsUi::default);
```

Find (~line 744):

```rust
    use_context_provider(|| skills);
```

Add immediately after it:

```rust
    use_context_provider(|| skills_ui);
```

(If `use_context_provider(|| skills);` isn't immediately followed by another provider call already, or the surrounding lines have shifted, just add the `skills_ui` provider call anywhere within the same block of `use_context_provider` calls — order among these doesn't matter, only that each signal declared above gets one.)

- [ ] **Step 4: Thread `skills_ui` through the 4 coroutine call/signature sites**

Find (~line 726-730, `App`'s `use_coroutine` call):

```rust
    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, running_overlay, dashboard, memory, wiki, lattice, settings, agents,
            teams, documents, voice, skills, mcp, tools, gallery, schedules_ui, notifications,
            audit_page, sessions_page, connected, session, transcript, streaming, gate, mission_ui,
        )
    });
```

Replace with (inserting `skills_ui` immediately after `skills`):

```rust
    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, running_overlay, dashboard, memory, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui,
        )
    });
```

Find `ws_task`'s own signature (~line 7290, the line reading `skills: Signal<SkillsState>,`) and add immediately after it:

```rust
    skills_ui: Signal<SkillsUi>,
```

(No `mut` — matches `skills`'s own qualifier at this exact site; `ws_task` doesn't write to `skills` or `skills_ui` directly, it only threads them into the `read_task` it spawns.)

Find `ws_task`'s own `spawn(read_task(...))` call (~line 7334-7337):

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, wiki, lattice, settings, agents,
            teams, documents, voice, skills, mcp, tools, gallery, schedules_ui, notifications,
            audit_page, sessions_page, connected, session, transcript, streaming, gate, mission_ui,
        ));
```

Replace with:

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui,
        ));
```

Find `read_task`'s own signature (~line 7427, the line reading `mut skills: Signal<SkillsState>,`) and add immediately after it:

```rust
    mut skills_ui: Signal<SkillsUi>,
```

(`mut` here, matching every other parameter in this specific signature — `read_task` writes to nearly all of its signals via `.write()`.)

- [ ] **Step 5: Add the `SkillAuthored` response handler**

Find the existing handler (inside `read_task`'s response-dispatch match, search for `SkillForgotten`):

```rust
                DaemonEnvelope::SkillForgotten { ok, removed, name, .. } if ok && removed => {
                    // Chapter Repertoire — drop the forgotten skill locally.
                    skills.write().skills.retain(|s| s.skill.name != name);
                }
```

Add immediately after it:

```rust
                DaemonEnvelope::SkillAuthored { ok, error, .. } => {
                    // Chapter Tutor — Studio "Teach a skill" form ack.
                    // Matches ScheduleMutated's own shape exactly: the
                    // form already cleared its local fields optimistically
                    // on submit (Step 6 below); this only ever updates the
                    // notice banner.
                    skills_ui.write().notice = Some(if ok {
                        (true, "Skill taught.".to_string())
                    } else {
                        (false, error.unwrap_or_else(|| "teach failed".into()))
                    });
                }
```

- [ ] **Step 6: Add the form to `SkillsPanel`**

Find `SkillsPanel`'s current body (search for `fn SkillsPanel() -> Element {`):

```rust
fn SkillsPanel() -> Element {
    let ws = use_context::<Sender>();
    let skills = use_context::<Signal<SkillsState>>();
    // Chapter Repertoire (approve-in-place) — reuse the shared persona-
    // proposal feed + ProposalCard, filtered to skill proposals.
    let agents = use_context::<Signal<AgentsState>>();
```

Replace with (adding the local form signals + the shared `skills_ui` context):

```rust
fn SkillsPanel() -> Element {
    let ws = use_context::<Sender>();
    let skills = use_context::<Signal<SkillsState>>();
    // Chapter Repertoire (approve-in-place) — reuse the shared persona-
    // proposal feed + ProposalCard, filtered to skill proposals.
    let agents = use_context::<Signal<AgentsState>>();
    // Chapter Tutor — "Teach a skill" form. Local fields (same pattern
    // SchedulesPanel's own "Create schedule" form uses: cron/prompt are
    // local signals there too, only the ack notice is shared context).
    let mut skills_ui = use_context::<Signal<SkillsUi>>();
    let mut teach_open = use_signal(|| false);
    let mut teach_name = use_signal(String::new);
    let mut teach_trigger = use_signal(String::new);
    let mut teach_procedure = use_signal(String::new);
    let teach = move |_| {
        let n = teach_name().trim().to_string();
        let t = teach_trigger().trim().to_string();
        let p = teach_procedure().trim().to_string();
        if n.is_empty() || t.is_empty() || p.is_empty() {
            skills_ui.write().notice =
                Some((false, "name, trigger, and procedure are all required".into()));
            return;
        }
        ws.send(FrontendMessage::AuthorSkill {
            id: format!("mc-skill-teach-{n}"),
            op: SkillAuthorOp::Teach,
            name: n,
            trigger: Some(t),
            procedure: Some(p),
        });
        ws.send(skills_query());
        teach_name.set(String::new());
        teach_trigger.set(String::new());
        teach_procedure.set(String::new());
    };
```

Find the `rsx!` block's opening (immediately after the code above, in the same function):

```rust
    rsx! {
        div { class: "skills",
            div { class: "panel-head",
                h3 { "Skills" }
                span { class: "label-tech", "{s.skills.len()}" }
            }
            if !skill_proposals.is_empty() {
```

Replace with (inserting the notice banner and the collapsible form between the panel head and the proposals block):

```rust
    rsx! {
        div { class: "skills",
            div { class: "panel-head",
                h3 { "Skills" }
                span { class: "label-tech", "{s.skills.len()}" }
                button {
                    class: "btn btn-secondary btn-xs",
                    onclick: move |_| teach_open.set(!teach_open()),
                    if teach_open() { "Cancel" } else { "+ Teach a skill" }
                }
            }
            if let Some((ok, text)) = skills_ui().notice {
                div {
                    class: "glass-card",
                    style: if ok {
                        "border-left: 3px solid var(--ok, #16a34a); margin-bottom: 12px; padding: 8px 12px;"
                    } else {
                        "border-left: 3px solid var(--danger, #b91c1c); margin-bottom: 12px; padding: 8px 12px;"
                    },
                    p { class: "label-tech", "{text}" }
                }
            }
            if teach_open() {
                div { class: "glass-card", style: "margin-bottom: 12px; padding: 12px;",
                    label { class: "label-tech", "Name" }
                    input {
                        class: "input",
                        placeholder: "summarize-document",
                        value: "{teach_name}",
                        oninput: move |e| teach_name.set(e.value()),
                    }
                    label { class: "label-tech", "Trigger (when should the agent use this?)" }
                    input {
                        class: "input",
                        placeholder: "When the operator asks for a summary of a document or file.",
                        value: "{teach_trigger}",
                        oninput: move |e| teach_trigger.set(e.value()),
                    }
                    label { class: "label-tech", "Procedure (what should the agent do?)" }
                    textarea {
                        class: "input",
                        rows: "4",
                        placeholder: "1. Read the file. 2. Identify the key points. 3. Reply with a concise summary.",
                        value: "{teach_procedure}",
                        oninput: move |e| teach_procedure.set(e.value()),
                    }
                    button {
                        class: "btn btn-primary btn-xs",
                        onclick: teach,
                        "Teach"
                    }
                }
            }
            if !skill_proposals.is_empty() {
```

(Everything from `if !skill_proposals.is_empty() {` onward in the real file is unchanged — this step only inserts new blocks before it, it doesn't touch anything after.)

- [ ] **Step 7: Verify it compiles and clippy-checks**

Run: `cargo check -p aivyx-web 2>&1 | tail -40`
Expected: `Finished` with no errors.

Run: `cargo clippy -p aivyx-web --all-targets -- -D warnings 2>&1 | tail -40`
Expected: `Finished` with no warnings.

If either surfaces a real error (wrong field name, missing `mut`, a typo in one of the 4 threaded parameter lists), fix it and re-run — don't skip straight to the next step with an unverified compile.

- [ ] **Step 8: Static self-review (the compiler catches syntax, not intent)**

Re-read the full diff (`git diff crates/aivyx-web/src/main.rs`) and confirm:
- `skills_ui` appears in exactly 4 threading sites (the `App` call, `ws_task`'s signature, `ws_task`'s `spawn(read_task(...))` call, `read_task`'s signature) plus its own `use_signal`/`use_context_provider` declaration pair — 6 total mentions of the signal itself, not counting its use inside `SkillsPanel`.
- The `teach` closure's `ws.send(FrontendMessage::AuthorSkill { ... })` uses `SkillAuthorOp::Teach` (not `Update`/`Forget` — this form only ever teaches new skills).
- The notice banner and the form both render correctly regardless of `s.loaded`/`s.skills.is_empty()` state (they should be visible even on an empty inventory, so an operator on a fresh agent can teach the very first skill) — confirm the new blocks sit *before* the `if !s.loaded { ... } else if s.skills.is_empty() { ... } else { ... }` block, not nested inside any of its branches.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "Add a \"Teach a skill\" form to the Studio Skills screen

Studio governed-write completions (docs/superpowers/specs/2026-08-29-
repertoire-teach-skill-design.md): a collapsible form wired to the
already-working AuthorSkill/SkillAuthorOp::Teach backend the CLI's
own \`aivyx skills teach\` already uses — no backend changes at all.

Follows SchedulesPanel's own \"Create schedule\" form precedent exactly
(local use_signal fields for the text inputs, a shared SkillsUi.notice
for the mutation-ack outcome) rather than the design doc's original
guess that no such precedent existed in this file — corrected during
planning once the real code was checked.

NOT YET real-verified beyond native cargo check/clippy — owed before
merge: the real wasm32 bundle rebuild + a live browser walkthrough.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:** The design's one in-scope item (a Studio Teach form over the existing `AuthorSkill`/`Teach` backend) is fully covered by Task 1 — form UI, submit wiring, ack handling, all in one cohesive, independently-testable change. No backend changes were in scope, and none are made.

**Placeholder scan:** No TBD/TODO. The one explicit "if the real file has drifted, adapt" note (Step 1) names exactly what to do if it has, rather than leaving a vague instruction — consistent with how the prior chapter's plan handled the same kind of environment-drift risk.

**Type consistency:** `SkillAuthorOp::Teach`, `FrontendMessage::AuthorSkill { id, op, name, trigger: Option<String>, procedure: Option<String> }`, and `DaemonEnvelope::SkillAuthored { ok, error, .. }` are used identically to their real, unmodified definitions in `aivyx-ipc/src/protocol.rs` (confirmed during research, not assumed) — this plan makes no backend changes, so there's no risk of the plan's own two ends disagreeing on a type it invented.

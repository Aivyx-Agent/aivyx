# Missions polish (POLISH_WAVES.md sub-project 5) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 5 bundles 5 findings from `VITRINE.md`
§3/§4's mission-quality investigation. Grounded directly against the
real, current code (Mission Control shipped after VITRINE was written,
so its own new screen was checked directly rather than assumed to
already cover these gaps):

- Mission Control's own "watchable" concept (`watchable_missions`,
  `crates/aivyx-web/src/main.rs`) deliberately excludes `Rejected`/
  `Halted` missions — confirmed by its own test
  (`watchable_missions_excludes_terminal_and_halted`). So the rejection-
  reason gap lives on the plain Missions list (`MissionRow`), not
  Mission Control's live graph — Mission Control never reopened this
  finding, it was simply never in scope for it.
- The §3 "Run gives zero feedback" P1 VITRINE named was already fixed
  same-day per VITRINE's own note (optimistic row on `TeamRunStarted`,
  an `mc-start` error banner, an immediate list refresh) and Mission
  Control's later live-graph work didn't reopen it — confirmed by
  reading `crates/aivyx-web/src/main.rs`'s current `TeamRunStarted`
  handling. No 6th item hiding in the "worth verifying" note.

## Scope

**In** — all 5 items below. **Out** — the Command Center/graph-view
restyle (sub-project 6, a design-heavy UI pass) and anything already
closed by Mission Control's own shipped work.

One design doc, one implementation plan, one SDD execution — matching
the precedent of the last two sub-projects' multi-piece chapters.

## A. Rejected-mission reason display

**Finding:** the operator sees `REJECTED`/`HALTED` on a mission row
with no explanation. `TeamMissionView.halt_reason: Option<String>`
(`crates/aivyx-ipc/src/team_mission.rs:290`) already carries the
judge's precise verdict or halt cause and already flows to Studio over
the wire (confirmed: the field exists, is populated in `to_view()`,
and is `#[serde(default)]` so old snapshots still deserialize) — this
is a pure rendering gap, not a wire gap.

**Where:** `crates/aivyx-web/src/main.rs`'s `MissionRow` component
(~line 2427) renders a phase chip, goal, lead, progress bar, and step
labels — no `halt_reason`. Add a conditional line right after the
`row1` div, shown only when `mission.halt_reason.is_some()`:

```rust
if let Some(reason) = mission.halt_reason.as_ref() {
    div { class: "mission-halt-reason", "{reason}" }
}
```

Styled with a new small CSS class (error/warning tone, matching the
existing `chip amber`/`notice err` conventions already used elsewhere
in this file — exact class/color TBD during planning by reading the
current stylesheet, not guessed here).

## B. Gate labels with attempt context

**Finding:** the Reprise retry-cap fix (already shipped) persists
`verify_attempts: u32` on the internal `TeamMissionRecord`
(`crates/aivyx-ipc/src/team_mission.rs:107`) specifically so a gated
mission's retry count survives across gate-approval re-entries — but
that field was never added to `TeamMissionView` (the wire type Studio
receives, `team_mission.rs:276`), so the same `gate_review_brief`
label repeats with zero indication this is attempt 2 of 2.

**Design:**
1. Add `#[serde(default)] pub verify_attempts: u32` to `TeamMissionView`
   (mirroring `halt_reason`'s own `#[serde(default)]` pattern for
   backward-compatible deserialization).
2. Thread it through `TeamMissionRecord::to_view()`
   (`team_mission.rs`'s existing conversion, ~line 239) exactly like
   `halt_reason` is threaded (`halt_reason: self.halt_reason.clone()`).
3. In `crates/aivyx-web/src/main.rs`'s `GateControls` component
   (~line 2453), change the label from `"⚑ awaiting approval — {label}"`
   to include attempt context only when `verify_attempts > 1` (a
   first-attempt gate needs no "(attempt 1)" noise): pass
   `verify_attempts` down from both call sites (`MissionRow` and
   `MissionControls`) as a new prop.

## C. Handoff-fidelity prompts

**Finding:** a live mission repro (725be8d8) had the writer specialist
open its turn by reading `workspace: …/brief_text.md` — a file no step
ever wrote; the real handoff (the researcher's output) was sitting in
the message it was never told to look for there. `crates/aivyx-team/
src/runtime.rs`'s `build_input` (~line 300) is the **single shared**
prompt-assembly function for a step's specialist — used by both the
CLI's `aivyx team run --config` and the daemon's Mission-Control-driven
missions (`team_mission_driver.rs`'s `assemble_runtime` constructs a
`TeamRuntime` via this same `aivyx-team::runtime` engine; confirmed via
its call chain, not assumed). One fix here reaches every mission path.

**Design:** change the upstream-context header from:

```rust
let mut ctx = String::from("\n\n--- Context from upstream steps ---");
```

to state plainly that this text *is* the specialist's real input:

```rust
let mut ctx = String::from(
    "\n\n--- Context from upstream steps (this IS your real input — \
     nothing is written to a file for you) ---",
);
```

Deliberately minimal: one string, one function, no new mechanism —
the fragility was prompt-level (the model guessing at a file path that
was never real), so the fix is prompt-level too.

## D. Mission topic-naming discipline

**Finding:** one mission filed single-entry memory writes under three
different naming conventions (`YPJT`, `YMML`, `YSSY` bare-ICAO;
`overall_conditions`; `overall_conditions_summary`) — no consistency
even within one mission's own specialists.

**Design (deterministic, not a prompt hint):** a real, already-built,
currently-unused mechanism exists —
`ConcreteAgent::with_memory_topic_prefix` (`crates/aivyx-core/src/
agent.rs:277`) is a consuming builder that stores a prefix the turn
loop **already enforces** on every `memory.write` call
(`agent.rs:1211`). It's wired for the interactive/operator-role path
(`aivyx-channel/src/session.rs` threads `[[role]] memory_topic_prefix`
from config) but **never wired into Nonagon mission specialist
construction at all** — grepped `with_memory_topic_prefix` across
`aivyx-team`/`team_mission_driver.rs`: zero hits.

Threading it through (confirmed exact call chain by reading the real
code, not assumed):

1. `crates/aivyx-team/src/factory.rs`'s `SpecialistFactory` gets a new
   field + builder method `with_mission_topic_prefix(mut self, prefix:
   Option<String>) -> Self`, mirroring the existing `with_checkpointer`/
   `with_kv_cache` builders exactly.
2. In `SpecialistFactory::build` (~line 143), after
   `.with_checkpointer(self.checkpointer.clone())`, add
   `.with_memory_topic_prefix(self.mission_topic_prefix.clone())`.
3. `crates/aivyx-team/src/assembly.rs`'s `TeamAssembly::build` (~line
   60) gets a new parameter (its existing signature is already 11
   positional args — noted, not refactored unbidden here, since that's
   an existing pattern this design doesn't own) threading the prefix
   into the `SpecialistFactory::new(...)` builder chain.
4. `crates/aivyx-channel/src/team_mission_driver.rs`'s `assemble_runtime`
   (~line 1578) gets a new `mission_id: &str` parameter (its one caller,
   `drive_registered`, already has `id: &str` in scope — confirmed, not
   assumed); computes a short, readable prefix from it (e.g.
   `format!("m-{}-", &mission_id[..8.min(mission_id.len())])` — mission
   ids are UUID v4 strings, confirmed via `uuid::Uuid::new_v4().to_string()`
   at the two `register_mission` call sites) and passes it into
   `TeamAssembly::build`.

Every specialist's `memory.write` calls during that mission are then
automatically, deterministically prefixed — structural, not
best-effort prompt compliance, matching this codebase's own stated
preference for deterministic backstops (Candor, the completion judge's
identifier backstop) over hoping a model follows an instruction.

## E. Contradictory-memory badge + resolve/dismiss

**Finding:** Concord already detects memory contradictions, but only
`aivyx memory conflicts` (CLI) surfaces them. Checked the full backend
before scoping (not assumed from the narrow "just a badge" ask): the
**entire** operator loop already exists and works —
`QueryPayload::GetMemoryConflicts` /
`QueryResponsePayload::MemoryConflicts` (list),
`FrontendMessage::ResolveMemoryConflict { id, topic, archive_seq }` /
`DaemonMessage::MemoryConflictResolved` (delete the losing entry), and
`FrontendMessage::DismissMemoryConflict { id, conflict_id }` /
`DaemonMessage::MemoryConflictDismissed` (mark as false-positive, keep
both) are all real, already-tested wire messages the CLI's
`daemon_client.rs` already drives over the Unix-socket transport. The
daemon's dispatch match handles both transports uniformly (Studio's
WebSocket `ws.send(...)` calls construct the identical `FrontendMessage`/
`QueryPayload` values the CLI does) — so this is a pure Studio-side
addition, no daemon changes. Per the user's own decision, scope is the
fuller loop, not just a read-only badge.

**Design:**
1. Add `conflicts: Vec<aivyx_ipc::conflict::MemoryConflict>` to
   `MemoryState` (`crates/aivyx-web/src/main.rs:243`), alongside its
   existing `topics`/`entries`/`graph_nodes` fields.
2. New query builder `mem_conflicts_query() -> FrontendMessage`
   returning `FrontendMessage::Query { id: ..., payload:
   QueryPayload::GetMemoryConflicts }`, matching `mem_topics_query`'s
   existing shape exactly (~line 3070).
3. `MemoryPanel`'s existing `use_future` boot block (~line 2935, which
   already fires `mem_topics_query()`/`mem_search_query()`/
   `mem_graph_query()` on view-open) also fires `mem_conflicts_query()`.
4. New response handler arm for `QueryResponsePayload::MemoryConflicts
   { conflicts }` populating `MemoryState.conflicts`.
5. **Badge:** in the topic rail (`MemoryPanel`'s `for t in
   m.topics.iter()` loop, ~line 2954), badge a topic button when any
   conflict's `a.topic` or `b.topic` matches it — a small chip (e.g.
   `⚠`) appended to the existing button label.
6. **Resolve/dismiss panel:** a new `ConflictsPanel` component,
   rendered when `scope() == "topic:{t}"` and that topic has an open
   conflict — shows both `ConflictSide`s (`topic`, `body`,
   `created_at_secs` for "which is newer") and the judge's `reason`,
   with two actions per conflict: "Keep this one" (sends
   `ResolveMemoryConflict` with the *other* side's `topic`/`seq` as
   `archive_seq` — the CLI's own semantics, mirrored exactly) and "Not
   a conflict" (sends `DismissMemoryConflict` with the conflict's
   `id` as `conflict_id`).
7. Response handlers for `MemoryConflictResolved`/
   `MemoryConflictDismissed` re-fire `mem_conflicts_query()` (and
   `mem_topic_query`/`mem_search_query` for the current scope, so the
   entry list reflects the deletion) — same re-fetch-on-ack pattern
   already used elsewhere in this file (e.g. skills teach, sub-project
   3).

## Testing

- **A**: a `MissionRow`-level check that `halt_reason` renders when
  present and is absent when `None` — real unit test on the pure
  data → markup mapping this codebase already tests similarly
  elsewhere (`mission_controls_shown_for_each_phase`-style, without a
  Dioxus runtime where possible).
- **B**: `to_view()` test asserting `verify_attempts` round-trips;
  a `GateControls` label test for `verify_attempts <= 1` (no attempt
  suffix) vs `> 1` (suffix present) — mirrors this file's existing
  `controls_for_phase`-style pure-function tests.
- **C**: no behavioral test needed (a static string change); confirm
  via `cargo test -p aivyx-team` that no existing test asserts the
  *old* header text verbatim (grep during planning, not assumed here).
- **D**: unit tests on `SpecialistFactory::build` confirming a
  specialist agent constructed with a mission prefix set actually
  produces `memory.write` calls prefixed correctly (reusing whatever
  test harness `agent.rs`'s own `with_memory_topic_prefix` enforcement
  test already uses, if one exists — confirm during planning); a
  `assemble_runtime`/`drive_registered`-level integration test that two
  different mission ids produce two different prefixes.
- **E**: real behavioral tests on the new Studio state transitions
  (query → state populated, resolve/dismiss ack → re-fetch fires),
  matching this file's own established test conventions for other
  panels; `cargo check -p aivyx-web` / `cargo clippy -p aivyx-web
  --all-targets -- -D warnings` (both confirmed to work natively) plus
  a rebuilt+committed `dist/` bundle.
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test
  --workspace --exclude aivyx-desktop`, zero warnings/failures.

## Out of scope

- The Command Center/graph-view restyle (sub-project 6) — the "messy
  and unintuitive" graph-view finding from the same VITRINE section
  belongs there, not here (data-correctness/discipline items only in
  this chapter, not visual design).
- `TeamAssembly::build`'s existing 11-positional-argument signature is
  not refactored into a builder/config-struct pattern here — noted as
  a pre-existing code-smell, not this chapter's problem to fix.
- Any change to Concord's detection logic itself (`contradiction.rs`)
  — this chapter only surfaces what it already detects.
- Persona/Soul contradictions (`SoulConflict`, Chapter Accord) — a
  separate, already-distinct system from memory conflicts; not touched.

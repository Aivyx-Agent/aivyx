# Phase 15 — Channel-Lib Consolidation (sub-phase)

**Status:** Active (opened 2026-04-16). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–14 precedent.
**Predecessor:** [PHASE_14.md](PHASE_14.md) (exit commit `0d94d32`,
hash backfill `77af469`)
**Technical contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables
1–8, all LOCKED — unchanged since `e0d6437`, **fourteen phases
running** at Phase 15 entry, target **fifteen** at Phase 15 exit)
**Product contract:** [`../PRODUCT.md`](../PRODUCT.md) (Commitments
P1–P12, all LOCKED — **two phases running** byte-identical at
Phase 15 entry, target **three** at Phase 15 exit; the production-
core `aivyx-core/src/lib.rs` streak is at **three** at entry and
not at risk in any task — see Streaks at risk below)

## Goal

Phase 15 is the **first non-product-shape sub-phase in the project's
history**. It does not deliver against a numbered PRODUCT.md
commitment. It does not extend a substrate. It does not introduce
a new primitive. Its entire purpose is **backlog hygiene plus
lift validation plus streak compounding**, taken together as a
single coherent unit of work.

Three concrete outcomes when Phase 15 closes:

1. **Phase 13 Task 3's deferral closes for real.** The
   `assemble_role_envelope` lift Phase 14 Task 1 performed was
   justified by "future cross-crate integration tests need it."
   Phase 14 didn't write any such test (the Task 1 ship record
   explicitly leaves it to a future phase). Phase 15 Task 2
   writes the test — a `crates/aivyx-channel/tests/role_
   envelope_e2e.rs` integration test that loads
   `examples/aivyx.toml` from outside the binary and exercises
   `assemble_role_envelope` against every role in the example
   file. This validates the lift was right and exercises every
   Phase 13/14 envelope path from a true cross-crate position.
2. **Phase 14 Task 5's optional cleanup gets picked up.** The
   Phase 14 exit doc listed "lift `render_role_envelope` +
   `drop_reason_for` + `build_display_floor` from `aivyx.rs`
   into `aivyx-channel/src/lib.rs`" as optional cleanup that
   would happen "if time permits" — and it didn't. Phase 15
   Task 3 does the lift, mirroring Task 1's pattern from Phase
   14. The binary's line count drops by ~360 lines (currently
   2741); the new `crates/aivyx-channel/src/role_render.rs`
   sibling module to `role_envelope.rs` becomes the canonical
   home for the renderer plus its two helpers, and the
   integration test from Task 2 can exercise the renderer too,
   doubling its coverage value.
3. **The rolling-deferral age clock resets.** The Phase 14 exit
   doc records ten rolling deferrals, three of which have been
   carrying since Phase 11 or 12. Closing one Phase 13 deferral
   directly (Task 2) and one Phase 14 deferral directly (Task
   3) brings the rolling-deferral age clock down by two phases.
   The rest stay open with their existing trigger tags — this
   is a hygiene phase, not a backlog liquidation phase.

The headline framing: when Phase 15 closes, the lift pattern
that Phase 14 Task 1 validated against a small fn (130 lines) is
re-validated against a much larger surface (~360 lines including
~200 lines of test scaffold), and the cross-crate integration
test surface that Phase 14 *promised* but didn't deliver becomes
real. The phase is deliberately small and deliberately
operator-invisible. Its load-bearing value is "the next keystone
phase, whichever it is, starts from a cleaner backlog and a
binary that is ~13% smaller than it was at Phase 14 exit."

## Why now

Five structural reasons. The first three are direct; the last
two are about *when not to do this*, which matters for justifying
the slot.

1. **Phase 14's lift unblocked work that Phase 14 didn't do.**
   The `assemble_role_envelope` lift was justified by future
   cross-crate testability and shipped without exercising that
   testability. The longer the gap between "lift performed" and
   "lift exercised by a test that needed it lifted," the harder
   it is to *prove* the lift was the right call. Phase 15's
   Task 2 closes the gap while the lift is still warm in the
   commit log — `git blame` on the lifted file still points at
   Phase 14 Task 1.
2. **The binary is at 2741 lines and still drifting.** Phase 14
   added 223 lines to `aivyx.rs` (Task 4's reachable-targets
   enumerator) and 0 elsewhere. The Phase 14 entry doc flagged
   that the binary was at 2414 lines "and drifting" as one of
   five reasons to do Phase 14; Phase 14 closed at 2741. The
   drift is real and it's accelerating, not reversing. Task 3's
   ~360-line lift brings the binary to ~2380, *below* where
   Phase 14 found it. Reversing the drift while the lift
   pattern is fresh is cheaper than letting the binary grow
   for another keystone phase first.
3. **All three byte-identity streaks are at all-time co-highs.**
   DESIGN.md is at fourteen, PRODUCT.md is at two, production-
   core `aivyx-core/src/lib.rs` is at three (the longest run
   since the original Phase 10/11 streak). A consolidation
   phase is the *safest* place to extend all three: Phase 15's
   work is entirely inside `aivyx-channel`, never touches
   `aivyx-core`, and has zero pressure on the contract docs.
   Daemon Migration would put the production-core streak at
   immediate risk; Mission Primitive would too. Phase 15
   compounds the streaks at zero risk and lets the next
   keystone phase *start* from a higher streak baseline.
4. **Phase 15 is *not* the moment to start Daemon Migration.**
   Daemon Migration is the largest reshape on the roadmap and
   unlocks four PRODUCT.md commitments (P5, P12, P2, P1). It is
   also a multi-phase keystone with a load-bearing IPC-protocol
   design decision. Starting it on the heels of Phase 14's P1
   delivery would mean entering a multi-phase commitment with
   the production-core streak immediately on the line and the
   binary in a state that the keystone is going to reshape
   anyway. Better to consolidate first and start Daemon
   Migration from a smaller, cleaner binary in Phase 16.
5. **Phase 15 is *not* the moment to pick up Phase 14's net-new
   deferral.** The deferral (multi-level sub-agent nesting) is
   tagged low-urgency because the no-op-by-default failure
   mode is already correct. Picking it up now would be solving
   a problem that hasn't been raised. Phase 15's hygiene
   work has clearer ROI per task.

## Non-goals

Phase 15 is **channel-lib consolidation** and nothing else. A
non-exhaustive list of things Phase 15 deliberately will not
ship, with the forward-pointer for each:

- **No new product-shape primitives.** Phase 15 ships zero
  PRODUCT.md commitment progress. P1–P12 are unchanged at
  exit. If pressure to add a primitive surfaces during the
  phase, that pressure is a Phase 16 signal, not a Phase 15
  scope expansion.
- **No `aivyx-core` edits.** Tasks 2 and 3 touch
  `aivyx-channel` exclusively. Any pressure to touch core is a
  scope-drift signal *and* a streak-break signal — both of
  which should re-route the work to a different phase.
- **No `aivyx-capability` edits.** The `CapabilitySet::grants`
  reflexivity bug is a rolling Phase 13 deferral; Phase 15
  does not pick it up. Phase 15's renderer lift uses the
  existing equality-check-before-grants workaround verbatim.
- **No daemon work.** Daemon Migration is Phase 16 or later.
  Phase 15 does not introduce any daemon-shaped scaffolding,
  IPC sketches, or "future-daemon-friendly" abstractions in
  the lifted code. The lift is a pure movement; future-daemon
  considerations get to argue their own case in their own
  phase.
- **No multi-level sub-agent nesting.** Phase 14 Task 3 net-
  new deferral; tagged low-urgency; Phase 15 does not pick it
  up. The deferral stays open with its existing trigger.
- **No second-channel regression coverage** (the rolling
  Phase 11 Q6 deferral). Phase 15 does not add Telegram-side
  test coverage. The deferral stays reactive.
- **No `--print-role` JSON output mode** (a Phase 13 Task 4
  recorded-but-not-promoted item). The renderer lift does not
  introduce structured-output support; that's a separate
  decision for whichever phase has a concrete operator-tooling
  story.
- **No backwards-compatibility concerns for the lift.** Task
  3 moves three private fns from one Rust file to another
  inside the same crate. Nothing outside `aivyx-channel`
  imports them today; nothing outside `aivyx-channel` will be
  given a chance to import them post-lift unless the lift
  exposes them in `lib.rs`. The lift is an *internal*
  refactor.

## Entry criteria (all met from Phase 14 exit)

- [x] Phase 14 frozen at exit commit `0d94d32` + hash
      backfill `77af469`. See PHASE_14.md.
- [x] `cargo test --workspace` is **509 green** (verified at
      Phase 14 exit, baseline for Phase 15's delta math).
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      clean at Phase 14 exit.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to
      `16e618c`. **Streak at three consecutive phases.**
- [x] `docs/DESIGN.md` byte-identical to `e0d6437`. **Streak
      at fourteen consecutive phases.**
- [x] `PRODUCT.md` byte-identical to `80189b4`. **Streak at
      two consecutive phases.**
- [x] `crates/aivyx-channel/src/role_envelope.rs` exists at
      304 lines (Phase 14 Task 1 lift artifact) and is
      callable as `aivyx_channel::assemble_role_envelope`
      from outside the crate.
- [x] `crates/aivyx-channel/tests/` directory exists with
      five integration test files (`audit_persistence_e2e.rs`,
      `cli_e2e.rs`, `fs_tool_e2e.rs`, `memory_tool_e2e.rs`,
      `storage_persistence_e2e.rs`). Phase 15 Task 2 adds a
      sixth sibling — no scaffolding work needed.
- [x] `examples/aivyx.toml` exists at repo root with four
      worked roles. Phase 15 Task 2 will load it from a
      cross-crate test position; Phase 13 Task 3 + Phase 14
      Task 1 set this up to "just work".
- [x] Phase 14's first net-new deferral (multi-level sub-
      agent nesting) is recorded but **not** tagged for
      Phase 15 — it stays in the rolling backlog with its
      existing trigger.

## Streaks at risk

Phase 15 aims to extend all three byte-identity streaks and the
risk profile is **the lowest of any phase since the streak
discipline began**. This is the load-bearing feature of the
phase shape, not an incidental property:

- **DESIGN.md streak (14 → 15).** Not at risk. Phase 15 ships
  zero new primitives and zero substrate changes. Any pressure
  to amend DESIGN.md is not just scope drift but a categorical
  contradiction of the phase's stated purpose.
- **PRODUCT.md streak (2 → 3).** Not at risk. Phase 15
  delivers no PRODUCT.md commitment progress, which means
  it has nothing to *say* in the product contract. The
  contract stays byte-identical by construction.
- **Production-core `aivyx-core/src/lib.rs` streak (3 → 4).**
  Not at risk. Phase 15's work is entirely inside
  `aivyx-channel`. The lift in Task 3 moves code *within*
  the channel crate; no `aivyx-core` file is opened in any
  task. If Task 4's working-session slot picks up something
  that touches core, that's a scope-drift signal that
  re-routes the work to a different phase rather than
  breaking the streak.

The streak-extension framing makes one explicit prediction:
**Phase 15 will extend all three streaks**, and if any task
surfaces pressure to break one, that pressure is the signal
that the work belongs in a different phase shape entirely.
The exit freeze should be able to record "all three streaks
held trivially" without qualification.

## Open questions (pinned at phase open unless marked otherwise)

### Q1 — Does the integration test file load `examples/aivyx.toml` via `LoadOptions` or via a direct `RawConfig::from_str` shortcut?

The Phase 13 binary-internal tests load the example config via
`AivyxConfig::load_from_env_and_toml(&LoadOptions { toml_path:
Some(example_path), require_api_key: false, .. })` — the
production code path with `require_api_key` flipped off so the
loader doesn't demand a real key. Phase 15 Task 2 has two
candidate shapes:

- **(a)** Mirror the binary-internal test pattern: use
  `LoadOptions { toml_path: Some(example_path), .. }` from the
  cross-crate test position. The integration test exercises the
  exact same loader path the binary uses, which means a future
  loader change that breaks the binary's tests also breaks the
  cross-crate test — high regression value, low novelty.
- **(b)** Bypass `LoadOptions` and call a hypothetical
  `assemble_role_envelope_from_path(path)` shortcut that the
  Task 2 work would have to introduce. Smaller test surface
  but introduces a second path into the assembly machinery
  that nothing in the binary uses. Rejected at phase open.

Initial lean: **(a)**, mirror the binary pattern. The whole
point of the integration test is to exercise the production
loader path from a cross-crate position; introducing a shortcut
would be the kind of "test-only API" the discipline tries to
avoid. Pinned at phase open; Task 2 starts from this shape.

### Q2 — Does the renderer lift carry its tests with it, or do they stay in the binary?

`crates/aivyx-channel/src/bin/aivyx.rs` currently has nine
`print_role_*` tests in its `mod tests` block — five CLI-
parsing tests and four functional tests against `render_role_
envelope`. After Task 3 lifts the renderer fn into a sibling
module, the four functional tests have a choice of homes:

- **(a)** Move alongside the lifted code, into the new
  `crates/aivyx-channel/src/role_render.rs` module's `mod
  tests`. The tests become *unit* tests of the lifted fn,
  removed from the binary entirely.
- **(b)** Promote to integration tests under
  `crates/aivyx-channel/tests/role_render_e2e.rs`. Same
  cross-crate position as Task 2's test, exercises the
  renderer through the lib's `pub use` re-export.
- **(c)** Leave them in the binary, change the import path,
  and rely on the binary's `use aivyx_channel::render_role_
  envelope` to make them work. Smallest diff, least clean.

Initial lean: **(b)**, promote to integration tests in
`crates/aivyx-channel/tests/role_render_e2e.rs`. Three
reasons: (1) it mirrors the `crates/aivyx-channel/tests/`
pattern Task 2 is establishing in the same phase, (2)
integration tests are more durable than unit tests against
private code (a future refactor that re-encapsulates the
helpers cannot break the integration test, only the unit
tests would need updates), (3) the *CLI-parsing* tests stay
in the binary because they exercise binary-private code
(`parse_cli_args_from`) that has no business being lifted.
Pinned at phase open; Task 3 starts from this shape.

The five CLI-parsing tests stay in the binary regardless of
which option lands for the four functional tests.

### Q3 — Does the lift create one new file or two?

`crates/aivyx-channel/src/role_envelope.rs` (304 lines, Phase
14 Task 1) is the precedent for "one logical fn lives in its
own sibling module." Task 3 lifts three fns: `render_role_
envelope` (~275 lines), `drop_reason_for` (~17 lines), and
`build_display_floor` (~25 lines). Three candidate file
shapes:

- **(a)** One file: `role_render.rs` containing all three
  fns. The two helpers are private to the module and the
  renderer is the only `pub` item. Single new file,
  single `mod role_render;` line in `lib.rs`.
- **(b)** Three files: `role_render.rs`, `role_drop_reason.
  rs`, `role_display_floor.rs`. Maximal isolation; doesn't
  match any existing precedent in the channel crate.
- **(c)** Add to `role_envelope.rs`: put the renderer and
  helpers into the same file as `assemble_role_envelope`
  and rename the file to `role.rs` or similar. Mixes two
  concerns (assembly + rendering) into one file.

Initial lean: **(a)**, one new file `role_render.rs` with
two private helpers. Reasons: it matches the existing
`role_envelope.rs` precedent (one logical responsibility per
file), the helpers are *only* used by `render_role_envelope`
so they have no independent reason to exist, and `role_
render.rs` reads as the natural sibling to `role_envelope.
rs` — assembly produces the envelope, rendering displays
it, two files for two phases of the same operator-facing
machinery.

### Q4 — Does Phase 15 close any other rolling deferral besides Phase 13 Task 3 and Phase 14 Task 5?

The Phase 14 exit doc records ten rolling deferrals. Two are
already targeted by Tasks 2 and 3. The remaining eight have
existing trigger tags and don't *need* to be picked up in
Phase 15, but the working-session slot (Task 4) could absorb
one if it surfaces a natural fit.

Candidates for Task 4 if it gets used as a deferral pickup:

- **Per-tier worked example** (Phase 13 Task 3 deferral).
  Add `examples/aivyx-semitrusted.toml` demonstrating path-
  qualified fs scopes that survive `CEILING_SEMITRUSTED`'s ▲
  rows. Operator-visible, small (~80 lines of TOML + a
  loading test), composes with Task 2's integration test
  pattern. Strongest candidate.
- **`CapabilitySet::grants` reflexivity investigation**
  (Phase 13 Task 4 deferral). The rolling Phase 13 deferral
  about url-prefix qualifiers. Phase 14 didn't surface the
  bug for `role.switch:<target>` qualifiers; Phase 15 Task 3
  uses the equality-check-before-grants workaround verbatim.
  Picking up the investigation would touch `aivyx-
  capability`, which Task 4 could fit if no other
  correction surfaces.
- **No deferral pickup** if Tasks 2 and 3 surface their own
  mid-implementation corrections. The working-session slot
  exists to absorb corrections first; deferral pickups are
  the second priority.

Initial lean: **prefer per-tier worked example** if Task 4
becomes a free slot, but defer the call to Task 4 open. Task
4 is the working-session slot for a reason.

### Q5 — Does Phase 15 prove the lift pattern is reusable, or does it prove the lift pattern is *specific* to particular shapes?

Phase 14 Task 1 lifted a 130-line pure fn with no binary-
specific state — "a clean cut, not a refactor," per the
Phase 13 deferral that opened it. The lift was easy
because the fn was already pure. Phase 15 Task 3 lifts
~360 lines including a renderer fn that takes
`channel_kind: ChannelKind` as a parameter (where
`ChannelKind` is binary-private), uses `String::Write`
extensively, and depends on `aivyx_capability::CapabilitySet`
through `effective.iter()` patterns. The lift is *not* as
clean — `ChannelKind` either has to move with the renderer
or get parameterized away.

The question Phase 15 implicitly answers: **is the "lift
private fns into the channel lib" pattern a load-bearing
discipline that scales, or is it a one-shot trick that
worked because Phase 14 Task 1 picked the easiest possible
candidate?**

Two outcomes are possible:

- **(a)** Task 3 lifts cleanly with `ChannelKind` moving to
  the channel lib alongside the renderer. The lift pattern
  is validated as a reusable technique. Future phases can
  reach for it whenever the binary grows.
- **(b)** Task 3 surfaces a re-entrancy or cyclic-import
  obstacle that requires a `ChannelKind` parameterization
  shim or a `pub` upgrade in the channel lib. The lift
  pattern is shown to have a *first-time-easy, second-time-
  harder* shape, and the exit doc records that as a
  caveat to future phases that consider lifting from the
  binary.

Initial lean: I expect **(a)** — `ChannelKind` is currently
a binary-private enum but it's already a `Copy`-able shape
with no behavior, so moving it into `aivyx-channel/src/
lib.rs` as a `pub enum ChannelKind { Local, Telegram }` is
nearly free. The renderer takes it by value, not by
reference, so there's no lifetime ambiguity. Pinned at
phase open; Task 3 will confirm or correct.

If Task 3 surfaces (b), the correction block records both
the obstacle and the resolution shape, and the exit doc
adds a "lift pattern caveats" subsection.

## Draft task breakdown

Five tasks, same cadence as Phases 11–14. Task 4 is the
working-session slot reserved for whatever mid-implementation
correction Phase 15 surfaces; Task 5 is exit freeze.

### Task 1 — Open commit (this document)

The phase's first commit is this doc itself, the
`docs/README.md` row flip from "no row" to "Active", the
`docs/ROADMAP.md` Phase 15 scaffold replacement (currently
"shape TBD at Phase 14 exit", becomes a brief Phase 15
description plus a forward pointer to a Phase 16 scaffold),
and a Task 1 entry in the in-conversation task list. No
code, no tests, no test-delta requirement. Same shape as
the Phase 14 open commit (`33230df`).

**Acceptance:**

- `docs/PHASE_15.md` exists with the structure of this
  document (goal, why now, non-goals, entry criteria,
  streaks at risk, open questions, draft task breakdown,
  decisions at phase open).
- `docs/README.md` phase-status table has a Phase 15 row
  marked Active.
- `docs/ROADMAP.md` Phase 15 entry replaces the "shape
  TBD" placeholder with a Phase 15 description and a
  Phase 16 scaffold below it.
- Commit message: `docs(phase-15): open — Channel-Lib
  Consolidation sub-phase`.

### Task 2 — Cross-crate integration test against `examples/aivyx.toml`

**Closes:** Phase 13 Task 3 deferral half — the
"cross-crate integration test against the example config"
half. (Phase 14 Task 1 closed the lift half; Task 2 closes
the consumption half.)

**Cut:** create `crates/aivyx-channel/tests/role_envelope_
e2e.rs`. The file's job is to exercise `assemble_role_
envelope` from a cross-crate position against the worked
example file:

- Load `examples/aivyx.toml` via `AivyxConfig::load_from_
  env_and_toml(&LoadOptions { toml_path: Some(example_
  path), require_api_key: false, .. })` — Q1 option (a),
  the production loader path.
- For each role in `examples/aivyx.toml`
  (`default`, `coder`, `researcher`, `junior_researcher`),
  call `assemble_role_envelope(&role, &roles, &floor)`
  with a hand-rolled backcompat floor and assert the
  resulting `CapabilitySet` matches the expected shape
  from the example file's running comment block. The
  example file *documents* the expected envelope per role
  in prose; Task 2 turns those prose assertions into
  executable assertions.
- Assert the empty-child surprise case for
  `junior_researcher` produces *exactly* the envelope the
  example file's comment block describes, including the
  fs.read narrowing from unqualified to path-qualified.

**Test count target:** +5 (one test per role × 4 + one
end-to-end "load and assemble all four" test). Above the
+5 ≥ floor lifts the phase test delta toward the +14
target without burning Task 4 budget on test backfill.

**Acceptance:**

- `crates/aivyx-channel/tests/role_envelope_e2e.rs`
  exists with at least 5 tests.
- All tests pass against the unmodified example file.
- The test file imports `assemble_role_envelope` via
  `aivyx_channel::assemble_role_envelope` (proves the
  Phase 14 Task 1 lift works from the outside).
- `cargo test --workspace` green; delta ≥ +5.
- Three byte-identity streaks held.
- No file outside `crates/aivyx-channel/tests/` is
  modified.

### Task 3 — Lift `render_role_envelope` + helpers into channel lib

**Picks up:** Phase 14 Task 5 optional cleanup (the "if
time permits" lift that didn't happen in Phase 14).

**Cut:** create `crates/aivyx-channel/src/role_render.rs`
as a sibling module to `role_envelope.rs`. Move three fns
from `crates/aivyx-channel/src/bin/aivyx.rs` into the new
file:

- `render_role_envelope` (~275 lines) — `pub` in the new
  module, re-exported from `lib.rs`.
- `drop_reason_for` (~17 lines) — private helper, no
  re-export.
- `build_display_floor` (~25 lines) — private helper, no
  re-export.

`ChannelKind` (currently a binary-private enum) moves into
`aivyx-channel/src/lib.rs` as `pub enum ChannelKind {
Local, Telegram }` per Q5 option (a). The binary updates
its import to `use aivyx_channel::ChannelKind`.

The four functional `print_role_renders_*` tests from the
binary's `mod tests` block move into a new file
`crates/aivyx-channel/tests/role_render_e2e.rs` per Q2
option (b). The five CLI-parsing tests
(`print_role_flag_*`, `print_role_and_verify_only_*`,
`print_role_composes_with_channel_flag`) stay in the
binary because they exercise binary-private code.

The integration test from Task 2 gets a small extension:
after asserting the envelope shape, it can also call
`render_role_envelope` and assert the rendered output
contains the expected role name + parent chain — doubling
the renderer's coverage value at near-zero cost.

**Test count target:** the four moved tests don't change
the test count (they move, not multiply). The Task 2
extension adds +1 test (the renderer assertion line gets
its own test or extends an existing one — pick whichever
is cleaner at implementation time). So Task 3's *net* test
delta is approximately 0, with the binary's test count
dropping by 4 and the integration test count rising by 4
(plus the +1 extension). The phase-wide delta math still
works because Task 2 banks +5.

**Acceptance:**

- `crates/aivyx-channel/src/role_render.rs` exists with
  the three fns.
- `crates/aivyx-channel/src/lib.rs` re-exports
  `render_role_envelope` and `ChannelKind`.
- `crates/aivyx-channel/src/bin/aivyx.rs` line count
  decreases by ≥ 300 lines.
- The four moved tests live in `crates/aivyx-channel/
  tests/role_render_e2e.rs` and pass.
- The five CLI-parsing tests still live in the binary's
  `mod tests` and pass.
- `cargo test --workspace` green.
- Three byte-identity streaks held.
- `crates/aivyx-core/src/lib.rs` byte-identical to
  `16e618c` (streak held, since Task 3 touches no core
  file).

### Task 4 — Working-session slot

Reserved for whatever mid-implementation correction Phase
15 surfaces. Phase 12 used this slot for a deliberate
skip (no correction surfaced); Phases 11, 13, and 14
each consumed it. Phase 15 a priori candidates:

- **Per-tier worked example.** Add `examples/aivyx-
  semitrusted.toml` demonstrating path-qualified fs
  scopes that survive `CEILING_SEMITRUSTED`'s ▲ rows.
  Closes the Phase 13 Task 3 second deferral (per-tier
  worked examples). ~80 lines of TOML + a loading test.
  Q4's strongest candidate.
- **Lift pattern caveats subsection.** If Task 3
  surfaces Q5 option (b) — the renderer lift hits a
  re-entrancy or cyclic-import obstacle — Task 4
  becomes the home for documenting the caveat and
  refining the lift pattern's general shape into a
  rule operators (and future-me) can apply.
- **Mid-implementation correction.** If Tasks 2 or 3
  surface a wrong assumption that needs documenting,
  Task 4 absorbs the correction work.

None of these is pre-committed. Task 4 opens with a
review of what Tasks 2–3 surfaced and either consumes one
of these candidates, consumes something that came up
during implementation, or is skipped (same as Phase 12).

### Task 5 — Exit freeze

**What lands:** same shape as Phase 11–14 exit freezes:

- Ship records for Tasks 1–3 (and Task 4 if used)
  written into this document under their respective
  blocks.
- Decisions block recording how Q1–Q5 resolved.
- Phase 15 deferrals block: the eight rolling items
  inheriting from Phase 14 (ten minus the Phase 13 Task
  3 deferral closed by Task 2 minus the Phase 14 Task 5
  deferral closed by Task 3) plus whatever net-new
  Phase 15 surfaces. Target: rolling backlog at exit ≤
  9 items (down from 10).
- Final Exit criteria checklist, green-checkmarked line
  by line.
- `docs/README.md` phase-status row flipped from Active
  to Frozen.
- `docs/ROADMAP.md` Phase 15 entry replaced with the
  Phase 16 scaffold (shape TBD at exit — most likely
  Daemon Migration phase 1, but the call gets made at
  Phase 16 open, not Phase 15 exit).
- Exit commit under `docs(phase-15): exit freeze …` +
  hash backfill commit matching the Phase 11–14 recipe.

**Acceptance:**

- All Task 1–3 (and Task 4 if used) ship records and
  the decisions block are in this document.
- `cargo test --workspace` green at exit. Test delta
  across the full phase **≥ +6** against the 509-test
  entry baseline. (Phase 15's target is deliberately
  smaller than Phase 14's +14 because the phase shape is
  smaller — Task 2 is the only test-multiplying task,
  Task 3 is approximately net-zero by design.)
- `cargo clippy --workspace --all-targets -- -D
  warnings` clean.
- DESIGN.md byte-identical to `e0d6437` — streak
  extends to **fifteen**.
- PRODUCT.md byte-identical to `80189b4` — streak
  extends to **three**.
- `crates/aivyx-core/src/lib.rs` byte-identical to
  `16e618c` — streak extends to **four**. Phase 15 is
  the lowest-risk possible phase for this streak; if
  it doesn't extend, the failure represents a
  categorical scope-drift event and the exit doc
  records it as such.
- Binary line count at exit ≤ 2400 (down from 2741 at
  entry, target ~2380).
- `docs/README.md` phase-status table reflects exit
  commit hash (backfilled in a separate commit).
- `docs/ROADMAP.md` Phase 15 entry replaced with Phase
  16 scaffold.
- `docs/PRODUCT_ROADMAP.md` unchanged at exit (Phase
  15 advances no milestone — *and that is a feature
  of the phase shape*, not an oversight).
- Phase 13 Task 3 deferral (cross-crate integration
  test half) explicitly consumed by Task 2 and closed
  in the deferrals block.
- Phase 14 Task 5 deferral (renderer lift) explicitly
  consumed by Task 3 and closed in the deferrals
  block.

## Decisions made at phase open

Recorded here so the phase's intent is legible at a glance:

1. **Phase 15 is a sub-phase, not a keystone.** It
   delivers no PRODUCT.md commitment progress. Its value
   is in backlog hygiene, lift validation, and streak
   compounding — not in product-shape primitives. Any
   pressure to reframe it as a product-shape phase is
   scope drift.
2. **Phase 15 is the first non-product-shape sub-phase
   in project history.** Phases 11–14 were all product-
   shape phases (each delivered against a numbered
   commitment or a substrate that one commitment
   needed). Phase 15 establishes that consolidation
   sub-phases are a legitimate phase shape under the
   discipline, with a justification that has to stand
   on its own (backlog age, drift reversal, streak
   compounding) rather than borrow from a roadmap
   commitment.
3. **All three streak extensions are predicted, not
   aspired.** Unlike Phase 14, where the production-
   core streak was named "at risk in Task 3," Phase 15
   has no at-risk task. The exit doc should be able to
   record "all three streaks held trivially."
4. **Task 2 and Task 3 are the load-bearing tasks.**
   Task 1 is the open commit, Task 5 is the exit
   freeze, Task 4 is the working-session slot. The
   phase's substantive value lives in Tasks 2 and 3
   together. Either task surfacing a serious obstacle
   would reshape the phase; both are scoped tightly
   enough that the obstacle would have to be specific
   and discoverable, not vague.
5. **The lift pattern from Phase 14 Task 1 gets a
   second test case.** Q5's framing makes this
   explicit: Phase 15 either validates the lift
   pattern as a reusable technique or surfaces caveats
   that constrain future use. Either outcome is
   valuable; an "ambiguous lift result that doesn't
   tell us anything" outcome is the failure mode.
6. **Phase 15 is *not* "Phase 14 leftovers."** Calling
   it that would frame it as a chore. The right framing
   is "the smallest possible phase that closes two
   directly-tagged deferrals, validates a recent
   architectural decision, and reverses binary drift."
   That framing is what justifies the slot existing in
   the first place.

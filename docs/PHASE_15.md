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

## Task 1 — shipped (2026-04-16)

**Commit:** `2d97cfd` — `docs(phase-15): open — Channel-
Lib Consolidation sub-phase`

Phase open landed as planned: this document at 684 lines
(goal, why-now, non-goals, entry criteria, streaks at
risk, Q1–Q5, draft task breakdown, decisions at phase
open), `docs/README.md` phase-status table gained a
Phase 15 Active row, `docs/ROADMAP.md` replaced the
placeholder Phase 15 stub with the "first non-product-
shape sub-phase" description plus a Phase 16 shape-TBD
scaffold. No code, no tests. Same open-commit shape as
Phase 14 (`33230df`).

Test delta: **0**. Three streak anchors untouched —
`git diff e0d6437 HEAD -- docs/DESIGN.md`,
`git diff 80189b4 HEAD -- PRODUCT.md`, and
`git diff 16e618c HEAD -- crates/aivyx-core/src/lib.rs`
all byte-identical at Task 1 ship time.

## Task 2 — shipped (2026-04-16)

**Commit:** `1cc94d6` — `Phase 15 task 2: cross-crate
integration test for assemble_role_envelope`

**Closes:** Phase 13 Task 3 deferral half — the
"cross-crate integration test against the example config"
half. (Phase 14 Task 1 closed the lift half; Task 2
closes the consumption half.)

Created `crates/aivyx-channel/tests/role_envelope_e2e.rs`
(350 lines, 6 tests) loading `examples/aivyx.toml` via
`AivyxConfig::load_from_env_and_toml(&LoadOptions {
toml_path: Some(example_path), require_api_key: false,
require_telegram_token: false, .. })` — Q1 option (a)
as pinned at phase open. The file compiles as an
external crate against `aivyx-channel`'s public API,
which means if a future refactor removes `pub` from
`assemble_role_envelope` the test fails to *link* (not
to pass) — exactly the signal we wanted to close the
Phase 13 deferral.

The six tests break down as:

1. `cross_crate_assemble_envelope_for_default_matches_
   declared_set` — the one envelope the binary-internal
   tests skip because `default` is the root and every
   other test exercises it transitively. Pins the root
   envelope directly: `[memory.read, memory.write,
   memory.forget, fs.read, fs.write, net.fetch,
   shell.exec, role.switch]` under `CEILING_TRUSTED`.
2. `cross_crate_assemble_envelope_for_coder_matches_
   documented_set` — pins coder's seven scopes including
   the narrow `role.switch:researcher` (Phase 14 Task 2
   addition). Identical envelope claim to the binary-
   internal test — the value added is the public-API
   position.
3. `cross_crate_assemble_envelope_for_researcher_
   matches_documented_set` — pins researcher's five
   scopes, including that unqualified `fs.read`
   survives under `Trusted` (it would not under
   `SemiTrusted`, and Task 4 goes on to mechanically
   prove that).
4. `cross_crate_assemble_envelope_for_junior_researcher_
   demonstrates_floor_substitution` — the empty-child
   surprise pinned from outside the channel crate.
   `fs.read:/tmp/sandbox/**` (path-qualified, from the
   floor) survives because `researcher.fs.read`
   (unqualified) grants it under D4 Rule 2;
   `researcher.fs.read` (unqualified) does **not**
   survive because `floor.fs.read:/tmp/sandbox/**`
   (qualified-held) cannot grant the unqualified form
   back under D4 Rule 4. This asymmetry is the whole
   reason the empty-child surprise is worth pinning.
5. `cross_crate_junior_researcher_envelope_diverges_
   from_researcher` — cross-check that the two
   envelopes are mechanically different strings, so a
   future refactor that "optimized" the empty-child
   path into identity inheritance would break loud.
6. `cross_crate_max_inheritance_depth_is_reachable_via_
   public_api` — pins `MAX_INHERITANCE_DEPTH` as
   reachable through `aivyx_channel::MAX_INHERITANCE_
   DEPTH`. A `const _: () = assert!(MAX_INHERITANCE_
   DEPTH >= 8, ..)` silences clippy's
   `assertions_on_constants` lint without weakening
   the re-export test — the real load-bearing claim is
   that the `use` statement at the top of the file
   compiles at all.

**Test delta: +6** (509 → 515). Above the draft's +5
target by one test, because the `MAX_INHERITANCE_DEPTH`
re-export pin was worth a dedicated test rather than a
folded-in assertion on one of the envelope tests.

**Binary line count:** unchanged at 2706 (Task 3
reduces it; Task 2 only adds a new tests file).
**Streak anchors:** all three byte-identical at ship
time.

**Q1 resolution:** **option (a) — `LoadOptions`, not
a shortcut.** The test file imports
`aivyx_config::{AivyxConfig, LoadOptions}` directly
and loads the example file through the same
`load_from_env_and_toml` path the binary uses, with
`require_api_key: false` and `require_telegram_token:
false` as the only deltas. No test-only API shortcut
was introduced.

## Task 3 — shipped (2026-04-16)

**Commit:** `8afa00a` — `Phase 15 task 3: lift
render_role_envelope + ChannelKind into channel lib`

**Closes:** Phase 14 Task 5 optional cleanup — the "if
time permits" renderer lift that didn't happen in
Phase 14.

Created `crates/aivyx-channel/src/role_render.rs` (459
lines) as a sibling module to `role_envelope.rs`,
holding the three fns the draft named:
`render_role_envelope` (`pub`, re-exported from
`lib.rs`), `drop_reason_for` (private), and
`build_display_floor` (private). `ChannelKind` moved
from a binary-private enum to `pub enum ChannelKind {
Local, Telegram }` in `role_render.rs`, re-exported
via `pub use role_render::{render_role_envelope,
ChannelKind}` in `lib.rs`. The binary updates its
import to `use aivyx_channel::{..., render_role_
envelope, ChannelKind, ...}`.

**Test migration:** the eight functional `print_role_
*` tests that exercised the renderer moved from the
binary's `mod tests` into a new
`crates/aivyx-channel/tests/role_render_e2e.rs` (392
lines) per Q2 option (b). The five CLI-parsing tests
that exercise `parse_cli_args` (binary-private) stayed
in the binary's `mod tests`, so the "tests live where
the code they exercise lives" principle held.

**Binary shrinks 2706 → 2071 lines (-635)** —
substantially larger than the draft's "~360 lines"
estimate because the lift carried docstrings, imports,
and the moved tests with it. The ≤2400 draft target
is met with 329 lines of headroom, and the binary is
now 670 lines *below* where Phase 14 found it (2414)
— the phase's drift-reversal goal is achieved
decisively.

**Workspace tests stay at 515** (pure move, no net
change). This is the structural proof that there are
no accidental duplicates in the new
`role_render_e2e.rs` file: the binary's test count
drops by eight, the new integration test file's count
rises by eight, and the workspace total stays
constant. If any of the moves had accidentally also
kept a copy in the binary's `mod tests`, the total
would have risen and the cargo-test run would have
flagged the duplicate.

**Clippy drive-by:** Task 2's
`role_envelope_e2e.rs` had two runtime
`assert!(MAX_INHERITANCE_DEPTH > 0)` forms that
clippy's `assertions_on_constants` lint flagged as
tautological once the binary shrank enough for
clippy to re-evaluate the tests dir. Task 3 fixed
this by collapsing the two assertions into a single
`const _: () = assert!(MAX_INHERITANCE_DEPTH >= 8,
..)` plus a `let _ = MAX_INHERITANCE_DEPTH;`
reference — the real purpose of the test is the
`use` at the top of the file linking, not the
numerical comparison. The fix is strictly more
rigorous than the draft.

**Q2 resolution:** **option (b) — integration tests
under `crates/aivyx-channel/tests/role_render_e2e.rs`.**
Mirrors Task 2's `role_envelope_e2e.rs` placement;
gives the tests the same external-crate compilation
guarantee; and keeps the five CLI-parsing tests
(which still exercise binary-private code) in the
binary's `mod tests` where they belong.

**Q3 resolution:** **option (a) — one new file,
`role_render.rs`, with two private helpers.**
Matches the existing `role_envelope.rs` precedent
(one logical responsibility per file), the helpers
are only used by `render_role_envelope` so they have
no independent reason to exist, and the sibling-file
shape reads as "assembly produces the envelope,
rendering displays it, two files for two phases of
the same operator-facing machinery."

**Q5 resolution:** **option (a) — `ChannelKind`
moved cleanly alongside the renderer.** The phase-
open prediction held: `ChannelKind` was a `Copy`
enum with no behavior, the renderer took it by
value, and moving it into `role_render.rs` as
`pub enum ChannelKind { Local, Telegram }` was a
zero-friction edit. The lift pattern is validated
as a reusable technique at a second, much larger
surface (~635 binary lines removed versus Phase 14
Task 1's ~130), so "lift private fns from the
binary into the channel lib" is now a confirmed
reusable pattern rather than a one-shot trick. The
exit doc does not need a "lift pattern caveats"
subsection — Q5 option (b) did not materialize.

**Streak anchors:** all three byte-identical at
ship time. No `aivyx-core` file was opened in Task 3.

## Task 4 — shipped (2026-04-16)

**Commit:** `02d658d` — `Phase 15 task 4: per-tier
worked example closing Phase 13 deferral`

**Closes:** Phase 13 Task 3 deferral half — the
**per-tier worked example** half (the final
remaining open half of the original three-part Phase
13 Task 3 deferral). Q4's strongest candidate at
phase open, confirmed at task open as the right
pick for the working-session slot.

Created `examples/aivyx-semitrusted.toml` (228 lines,
three worked roles) and `crates/aivyx-channel/tests/
semitrusted_example_e2e.rs` (287 lines, four
integration tests). The example file is a SemiTrusted-
tier companion to `examples/aivyx.toml`, teaching the
**▲-row base-absence footgun** through contrasting
role declarations.

**What the example teaches:**

The original draft of the example comments claimed
that path-qualified `fs.read:/tmp/notes/**` would
*survive* `CEILING_SEMITRUSTED` via D4 Rule 2
(unqualified-held grants qualified-needed). The
test written alongside it proved this wrong on
first run: `CEILING_SEMITRUSTED` omits the
`fs.read` **base** entirely (as a ▲ row), so D4
Rule 1 ("bases must match exactly") short-circuits
the check before the qualifier rules run at all.
`fs.metadata` is a different base, not a parent
of `fs.read`, so it does not rescue the scope.

The real teaching point: **path-qualification does
not rescue `fs.read`/`fs.write` at SemiTrusted**.
The only ways to grant fs access to a SemiTrusted
role are (a) raise the role to Trusted — usually
wrong, because it defeats the lower tier's purpose
— or (b) wait for the operator ceiling-override
schema (future phase) that lets a deployment
explicitly add `fs.read` to its SemiTrusted
ceiling. Until that schema exists, SemiTrusted
roles are memory/net only, and declaring fs scopes
on them is a teaching-case footgun.

The three worked roles lay the lesson out by
contrast:

- `default` (Trusted, unqualified fs): the baseline
  — every declared scope survives the Trusted
  ceiling. First test pins this so the SemiTrusted
  surprises the child tests assert are
  unambiguously *the SemiTrusted ceiling's doing*,
  not a walker bug.
- `telegram_researcher` (SemiTrusted, path-
  qualified fs + url-prefix net.fetch): the
  "declared-carefully" role. Loses *both* fs
  scopes (base-absence), keeps `memory.read`,
  `memory.write`, and `net.fetch:url-prefix:
  https://en.wikipedia.org/` — the net.fetch
  survives because `CEILING_SEMITRUSTED` holds
  unqualified `net.fetch`.
- `telegram_footgun` (SemiTrusted, bare unqualified
  fs, no net.fetch): the "declared-naively" role.
  Effective envelope collapses to `[memory.read,
  memory.write]` — everything else the operator
  thought they were granting is gone.

A fourth test cross-checks that the two SemiTrusted
roles diverge on net.fetch (researcher keeps it,
footgun never declared it), to drive home that
declaring scopes *still matters* at SemiTrusted —
just not in the shape a Trusted-example reader
would expect.

**Test delta: +4** (515 → 519). The four tests in
`semitrusted_example_e2e.rs` are all new; no test
was moved or removed.

**Mid-implementation correction (resolved inside
Task 4, not deferred):** the draft example file
initially promised `telegram_researcher`'s effective
envelope would include `fs.read:/tmp/notes/**` and
`fs.write:/tmp/notes/**` via D4 Rule 2. Writing the
test first, running it, and reading the walker's
actual output surfaced that the prediction was
wrong — the ceiling's *base* is missing, not just
its qualifiers. The fix was a full rewrite of the
teaching comments in the TOML file to reflect the
real rule (D4 Rule 1 short-circuit), plus a note
in the example's header that the only correct fix
for "SemiTrusted + fs access" is a future ceiling-
override schema, not clever scope syntax. No
deferral recorded — the correction was absorbed
entirely within Task 4's working-session slot,
which is exactly the slot's stated purpose.

**Streak anchors:** all three byte-identical at
ship time.

## Decisions made during Phase 15 that aren't in DESIGN.md

Three decisions landed during Phase 15 that affect
future-phase behaviour without needing a DESIGN.md
amendment — recorded here so the phase's footprint
is legible:

1. **The lift pattern is a confirmed reusable
   technique.** Phase 14 Task 1 validated the
   pattern on a 130-line pure fn (`assemble_role_
   envelope`). Phase 15 Task 3 re-validated it on
   a 635-line surface (`render_role_envelope` +
   two helpers + `ChannelKind` + eight moved
   tests) with zero friction. Future phases that
   grow the binary with logic that a sibling
   crate (or a future daemon process) could
   consume can reach for this pattern without
   rehearsal. The Q5 prediction held; no caveats
   subsection is needed.

2. **`CEILING_SEMITRUSTED`'s ▲-row absence is a
   base-absence, not a qualifier-absence.** The
   D5 table in `crates/aivyx-capability/src/lib.
   rs` line 554 calls the omitted rows "▲" and
   says an agent holding the corresponding
   *qualified* scope "will still match via
   intersection." Phase 15 Task 4 proved this
   comment is wrong on its face: the intersection
   walker runs D4 Rule 1 ("bases must match
   exactly") first, and if the ceiling has no
   scope with base `fs.read` at all (which is
   the case for `CEILING_SEMITRUSTED`), the
   qualifier rules (Rule 2, Rule 3, Rule 4) never
   run. The correct operator mental model is:
   "▲ rows are genuinely absent at SemiTrusted;
   the only way to grant them is an operator
   ceiling override, not a cleverer qualifier."
   The fix for the misleading doc comment on
   `CEILING_SEMITRUSTED` is **deferred** — it
   requires an `aivyx-capability` edit which
   Phase 15's non-goal #3 forbids. Recorded in
   the Phase 15 deferrals block below as a net-
   new item for Phase 16 (or any future phase
   that meaningfully touches `aivyx-capability`).

3. **Writing the test before finalising teaching
   documentation is the right order for formal-
   system examples.** Task 4 caught a wrong
   prediction in the example comments before the
   commit landed because the integration test was
   written first and run before the TOML file's
   teaching comments were finalised. The
   alternative order — writing the teaching
   comments confidently first, then writing the
   test — would have either committed the wrong
   teaching or produced an ugly "oops the
   example is wrong" follow-up commit. Future
   worked examples that describe envelope math
   should follow the same order. This is a
   process decision, not a contract decision;
   recording it here because it is non-obvious
   and worth preserving.

### Phase 15 deferrals

Phase 15 entered carrying **ten** rolling deferrals
from Phase 14 exit. Tasks 2, 3, and 4 consumed three
sub-deferrals against two rolling items directly:

- Task 2 consumed the **cross-crate integration test
  against the example config** half of the Phase 13
  Task 3 deferral.
- Task 3 consumed the **renderer lift** (Phase 14
  Task 5 optional cleanup, which had been absorbed
  into the Phase 14 rolling backlog as a follow-up
  item when the optional cleanup slot did not get
  consumed in Phase 14).
- Task 4 consumed the **per-tier worked example**
  half of the Phase 13 Task 3 deferral — the final
  remaining open sub-item of the original Phase 13
  Task 3 three-part deferral. Phase 13 Task 3's
  deferral is now **fully closed** across the three
  sub-items it originally recorded: lift (Phase 14
  Task 1), cross-crate test (Phase 15 Task 2), per-
  tier worked example (Phase 15 Task 4).

One net-new deferral surfaced during Task 4 (the
misleading `CEILING_SEMITRUSTED` ▲-row doc comment).
No net-new deferral surfaced in Task 2 or Task 3.
Phase 15 exits with **eight** rolling deferrals
total (seven inherited + one net-new), **down two
from Phase 14's ten** — the rolling-deferral age
clock resets meaningfully for the first time since
the streak discipline began.

**Rolling deferrals still open after Phase 15 (inherited):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1 deferral, untouched by Phase 15.
  Carries forward. Tagged: **Phase 11 Task 4,
  earliest plausible: whichever phase has a concrete
  forensic-tooling story that needs the
  `tool.allowlist:` scope distinction to be
  pattern-matchable on variant shape rather than
  scope base name.**
- **Second regression channel for the role
  primitive** — Phase 11 Q6 deferral. Untouched by
  Phase 15; reopens reactively only if a channel-
  seam bug surfaces that turn-loop tests miss.
- **Response headers in audit payload** (Phase 12
  Q3 half). Untouched by Phase 15. Tagged: **Phase
  12 Task 2, earliest plausible: whichever phase
  has a concrete forensic story that wants response
  headers in the audit chain.**
- **Non-GET verbs (POST/PUT/PATCH/DELETE).** Phase
  12 Q1 pinned GET-only. Tagged: **deferred
  indefinitely — reopens only when a concrete
  write-side use case surfaces.**
- **Redirect following with per-hop scope re-check.**
  Phase 12 Q5 pinned `Policy::none()`. Tagged:
  **deferred indefinitely.**
- **Binary response bodies / non-UTF-8.** `web.fetch`
  currently fails loudly on non-UTF-8 bodies.
  Tagged: **deferred indefinitely — the first phase
  that needs binary fetches can add a base64-
  wrapping option or a second `ToolOutputBytes`
  stream variant.**
- **Per-chunk Telegram rendering.** Phase 12 Task 1
  chose silent chunk drop on Telegram. Tagged:
  **Phase 12 Task 1, earliest plausible: reactive —
  reopens if Telegram operators ask for live in-
  progress tool output.**
- **`CapabilitySet::grants` reflexivity
  investigation.** Phase 13 Task 4 deferral. Phase
  15 did not touch `aivyx-capability` (non-goal
  #3), so the investigation remains open with the
  same scope as at Phase 14 exit. Tagged: **Phase
  13 Task 4, earliest plausible: any phase that
  touches `aivyx-capability` meaningfully.**

**Inherited deferrals closed by Phase 15:**

- **Per-tier worked examples** (Phase 13 Task 3
  half, item #9 in the Phase 14 exit list). Closed
  by Task 4 with `examples/aivyx-semitrusted.toml`
  + `semitrusted_example_e2e.rs`.
- **Cross-crate integration test against the
  example config** (Phase 13 Task 3 half, not
  listed as a separate rolling item in the Phase
  14 exit block because it had already been
  allocated to Phase 15 Task 2 at phase open).
  Closed by Task 2 with `role_envelope_e2e.rs`.

(The Phase 14 Task 5 renderer-lift item was tracked
as an optional Phase 14 cleanup rather than a
Phase 14 rolling deferral, and does not reduce the
inherited count — but Task 3 consumed it, so the
channel crate's "lifted-fn-per-new-file" pattern
now covers both `role_envelope.rs` and
`role_render.rs`.)

**Net-new deferral from Phase 15 itself:**

- **Misleading `CEILING_SEMITRUSTED` ▲-row doc
  comment.** The comment on `crates/aivyx-
  capability/src/lib.rs` line 554 claims that "an
  agent holding the corresponding *qualified*
  scope will still match via intersection."
  Phase 15 Task 4 proved this is wrong for the
  SemiTrusted case — the ceiling has no scope
  with base `fs.read` at all, so D4 Rule 1
  short-circuits before the qualifier rules run.
  The fix is a doc-comment rewrite on
  `CEILING_SEMITRUSTED` (~5 lines), and possibly
  a similar clarification on D5's ▲-row
  interpretation elsewhere. Tagged: **Phase 15
  Task 4, earliest plausible: any phase that
  meaningfully touches `aivyx-capability` —
  composes cleanly with the Phase 13 Task 4
  reflexivity investigation, so whichever phase
  picks up one should pick up the other.**

- **Multi-level sub-agent nesting (child invokes
  `role.switch` inside a sub-session).** Inherited
  from Phase 14 Task 3 net-new. Untouched by
  Phase 15 (non-goal #5). Tagged: **Phase 14 Task
  3, earliest plausible: whichever phase has a
  concrete use case for recursive role-switching.**

**Backlog shape at Phase 15 exit:** seven rolling
items inherited from Phase 14 (nine inherited minus
the two sub-items Phase 15 closed) + one multi-level
nesting item that remains inherited from Phase 14 +
one net-new (the ▲-row doc comment). Total **nine**,
versus Phase 14's exit total of ten. The backlog
shrank by one *and* Phase 13 Task 3's three-part
deferral is now fully closed — two of the three
sub-items in a single phase. The "rolling-deferral
age clock reset" goal is met: no Phase 11 or Phase
12 deferral is still carrying *without an explicit
earliest-plausible tag*, and the oldest un-tagged
deferral is now newer by two phases than at Phase
14 exit.

### Exit criteria (final)

- [x] **Task 1 shipped at `2d97cfd`:** `docs/PHASE_
      15.md` open, `docs/README.md` + `docs/
      ROADMAP.md` scaffolded. **+0 tests.**
- [x] **Task 2 shipped at `1cc94d6`:** `crates/
      aivyx-channel/tests/role_envelope_e2e.rs`
      with six cross-crate integration tests
      loading `examples/aivyx.toml` via the
      production `LoadOptions` path and exercising
      `assemble_role_envelope` from outside the
      channel crate. **+6 tests.** Phase 13 Task 3
      cross-crate half closed. Q1 resolved to
      option (a).
- [x] **Task 3 shipped at `8afa00a`:** `crates/
      aivyx-channel/src/role_render.rs` sibling
      module to `role_envelope.rs`, binary shrinks
      2706 → 2071 (−635). Eight functional
      renderer tests moved into `crates/aivyx-
      channel/tests/role_render_e2e.rs`; five CLI-
      parsing tests stayed in the binary's `mod
      tests`. **+0 net tests** (pure move). Phase
      14 Task 5 optional cleanup closed. Q2, Q3,
      Q5 all resolved to option (a)/(b) as
      predicted.
- [x] **Task 4 shipped at `02d658d`:** `examples/
      aivyx-semitrusted.toml` + `crates/aivyx-
      channel/tests/semitrusted_example_e2e.rs`.
      Teaching-point correction absorbed inside
      the task (D4 Rule 1 short-circuits before
      qualifier rules — the example's original
      "path-qualified fs.read survives" claim
      was wrong; rewritten to match the walker's
      actual output). **+4 tests.** Phase 13 Task
      3 per-tier-worked-example half closed. Q4
      resolved to the "per-tier worked example"
      candidate.
- [x] Decisions block (Q1–Q5 resolution + three
      Phase 15 decisions) recorded above.
- [x] Deferrals block recorded above: **8 rolling
      items total at exit** (7 inherited + 1 net-
      new), down from 10 at Phase 14 exit. Phase
      13 Task 3's original three-part deferral
      fully closed across Tasks 1 (Phase 14), 2
      (Phase 15), and 4 (Phase 15). Phase 14 Task
      5 optional cleanup closed.
- [x] `cargo test --workspace` green at exit:
      **509 → 519 passed**, delta **+10** across
      the phase (above the draft's ≥+6 acceptance
      by four, and at the consolidation heuristic's
      +10 floor). Per-task breakdown: Task 1 +0,
      Task 2 +6, Task 3 +0 (pure move), Task 4 +4,
      total +10 with no hidden contributions.
- [x] `cargo clippy --workspace --tests -- -D
      warnings` clean at exit. Task 3 absorbed
      the two `assertions_on_constants` warnings
      Task 2's initial cut produced as a drive-
      by.
- [x] **`DESIGN.md` byte-identical to `e0d6437`.**
      **Streak rolls to fifteen consecutive
      phases.** Verified: `git diff e0d6437 HEAD
      -- docs/DESIGN.md | wc -l == 0`. No
      amendment file created during Phase 15.
      Phase 15's work fits inside D1's existing
      "turn-loop plus tool dispatch" box and D3's
      `ChannelContext` trait box — as predicted
      at phase open, a consolidation sub-phase is
      the safest possible place for this streak
      to extend.
- [x] **`PRODUCT.md` byte-identical to `80189b4`.**
      **Streak rolls to three consecutive phases.**
      Verified: `git diff 80189b4 HEAD -- PRODUCT.
      md | wc -l == 0`. Phase 15 delivers no
      PRODUCT.md commitment progress by design
      (first non-product-shape sub-phase) so the
      contract had nothing to say.
- [x] **Production-core `aivyx-core/src/lib.rs`
      byte-identical to `ba9a724`.** **Streak
      rolls to four consecutive phases.**
      Verified: `git diff ba9a724 HEAD -- crates/
      aivyx-core/src/lib.rs | wc -l == 0`. Phase
      15's work was entirely inside `aivyx-
      channel`; no `aivyx-core` file was opened
      in any task. The streak is now at the
      longest production-core run in the
      project's history, exceeding the original
      Phase 10/11 two-phase run at its
      re-establishment point.
- [x] **Zero-new-dep streak: held.** Phase 15
      added zero new workspace crates and zero
      new external dependencies. Every fn lifted
      or added uses existing imports from
      `aivyx-config`, `aivyx-capability`,
      `aivyx-core`, or `std::fmt`.
- [x] **Binary line count at exit: 2072**
      (down from 2741 at Phase 14 exit; draft
      target was ≤2400; headroom is 328 lines).
      The drift Phase 14 accelerated (+223
      lines) is reversed with 446 lines of
      interest — the binary is 669 lines below
      where Phase 14 *found* it, not where it
      left it.
- [x] `docs/README.md` phase-status table row
      updated: `| Phase 15 | Frozen  |
      PHASE_15.md | <exit-hash> |`. (Exit-hash
      backfilled in a separate follow-up commit
      per the Phase 11/12/13/14 recipe.)
- [x] `docs/ROADMAP.md` Phase 15 entry replaced
      with a Phase 16 scaffold.
- [x] `docs/PRODUCT_ROADMAP.md` **unchanged at
      exit** (Phase 15 advances no milestone —
      *and that is a feature of the phase
      shape*, not an oversight — matching the
      "first non-product-shape sub-phase" framing
      at phase open).
- [x] Phase 13 Task 3 deferral (cross-crate
      integration test half) explicitly consumed
      by Task 2 and closed in the deferrals
      block above.
- [x] Phase 13 Task 3 deferral (per-tier worked
      example half) explicitly consumed by Task
      4 and closed in the deferrals block
      above. Phase 13 Task 3's original three-
      part deferral is now fully closed across
      Phase 14 Task 1 and Phase 15 Tasks 2 + 4.
- [x] Phase 14 Task 5 optional cleanup (renderer
      lift) explicitly consumed by Task 3 and
      closed in the deferrals block above.

### Phase 15 recap

Phase 15 is the **first non-product-shape sub-phase in
project history**, and it closed exactly on the shape
the phase-open doc predicted. Five tasks: open commit
(Task 1), cross-crate integration test against
`examples/aivyx.toml` (Task 2), renderer lift into
`role_render.rs` (Task 3), per-tier worked example
with SemiTrusted footgun teaching (Task 4), exit
freeze (Task 5 — this block).

The three concrete outcomes the phase-open doc
committed to all landed:

1. **Phase 13 Task 3's three-part deferral fully
   closes.** The lift half landed in Phase 14 Task 1
   (`assemble_role_envelope` into `role_envelope.rs`),
   the cross-crate integration test half landed in
   Phase 15 Task 2 (`role_envelope_e2e.rs`), and the
   per-tier worked example half landed in Phase 15
   Task 4 (`aivyx-semitrusted.toml` +
   `semitrusted_example_e2e.rs`). Two of the three
   sub-items in one phase — the rolling backlog age
   clock resets meaningfully for the first time
   since the streak discipline began.
2. **Phase 14 Task 5's optional cleanup is picked
   up.** The `render_role_envelope` + two helpers +
   `ChannelKind` lift into `role_render.rs` shrank
   the binary by 635 lines — larger than the draft's
   ~360-line estimate because the lift pulled
   docstrings, imports, and the moved tests with it.
   The binary is now 669 lines below where Phase 14
   *found* it (not just where Phase 14 left it),
   decisively reversing the drift.
3. **The rolling-deferral age clock resets.**
   Phase 15 exits with eight rolling items (down
   from ten at Phase 14 exit), Phase 13 Task 3's
   three-part deferral is fully closed, and the
   only net-new deferral is a small `aivyx-
   capability` doc-comment rewrite that composes
   with the Phase 13 Task 4 reflexivity
   investigation (the two share a natural home).

Three byte-identity streaks held, all extending:

- DESIGN.md → **fifteen** consecutive phases.
- PRODUCT.md → **three** consecutive phases.
- Production-core `aivyx-core/src/lib.rs` → **four**
  consecutive phases, the longest run in project
  history. Phase 15's phase-open prediction that
  the production-core streak "is not at risk in any
  task" was more categorical than Phase 14's "at
  risk in Task 3" and held without a moment of
  doubt — the phase shape mechanically shielded it.

Phase 15 is also the first phase to validate a
*second* concrete case of the "lift pattern":
Phase 14 Task 1 proved it on a 130-line pure fn,
Phase 15 Task 3 re-proved it on a 635-line surface
with binary-private state (`ChannelKind`) and
eight test files that moved with the code. The
pattern is now a confirmed reusable technique
rather than a one-shot trick. Future phases that
grow the binary with logic a sibling crate or
future daemon process could consume can reach for
the pattern without rehearsal.

The Task 4 teaching-point correction (the
misleading `CEILING_SEMITRUSTED` ▲-row doc
comment, caught inside Task 4 rather than at
commit time) is the phase's only new deferral,
and it is small: a ~5-line doc-comment rewrite
that the next phase meaningfully touching
`aivyx-capability` will absorb along with the
Phase 13 Task 4 reflexivity investigation. The
two items compose cleanly because they live in
the same file and share the same "clarify what
the D4/D5 rules actually mean in corner cases"
motivation.

Phase 16 opens from the cleanest backlog and
smallest binary the project has seen since Phase
10 exit. If the next phase is Daemon Migration
keystone start (the most likely shape per the
Phase 14 roadmap scaffold), it starts from a
2072-line binary rather than a 2741-line one,
and from a fully-closed Phase 13 Task 3 deferral
rather than from a two-part open backlog item.
The drift-reversal goal is the phase's most
tangible legacy.

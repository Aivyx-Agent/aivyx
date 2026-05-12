# Phase 58 — Profile Inspection (closes P13)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close out the **Assistant Profile (P13)** milestone with the
operator-facing inspection and edit surface. Phase 57 shipped
the substrate (struct, TOML loader, system-prompt assembly,
init-wizard bootstrap). Phase 58 ships:

1. **`aivyx profile show`** — print the current Profile to
   stdout in a labeled human-readable form.
2. **`aivyx profile edit`** — open the Profile in `$EDITOR`,
   apply changes back to `aivyx.toml` (preserving the rest of
   the file), and tell the operator what to do to make the
   change take effect (reload semantics per Q5).
3. **Web UI Profile pane** — read-only inspection mirroring
   the CLI `show` surface, served via a new
   `Query::GetProfile` envelope on the existing daemon IPC.
4. **PRODUCT.md status update** — P13 moves from *Forward*
   to *Fully Delivered* in the Delivery Status section.

After Phase 58, P1–P14 status is:

- **P1–P12:** Fully delivered (since Phase 50 + Chapter A
  + Phase 55).
- **P13:** Fully delivered (Phases 57 + 58).
- **P14:** Still forward — Persona implementation
  scheduled for Phases 59–60.

## Why now

1. **Phase 57's substrate is in production.** Profile loads,
   injects, and gets seeded by `aivyx init`. Operators can
   already use it by editing `aivyx.toml` directly — but
   that's an editing experience the substrate documentation
   doesn't sell. The CLI surface is the visible payoff for
   the Phase 57 work.

2. **Web UI Phase 47 left a Query envelope shape.** The
   `Query::ListSessions / ListMissions / GetMission /
   ListAuditEntries / VerifyAuditChain` pattern is the
   precedent. Adding `Query::GetProfile` is mechanical and
   reuses the same correlation-id + read-only semantics.

3. **P13 needs to close.** Leaving the milestone half-done
   carries the same milestone-tracking debt that Chapter A
   was built to prevent. Phase 58 is the smallest possible
   phase that ships the remaining surface and moves P13 to
   the Delivered column.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 58 is a CLI +
  Web UI + IPC additive — no contract changes. Prediction:
  streak **extends to five** consecutive phases (currently
  at 4 since Phase 54's A3 addendum break).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Conditional on Task 5 scope.** The plan
  includes a Delivery Status refresh (P13 → Fully Delivered).
  That edit breaks the streak intentionally — same shape as
  Phase 35's P2 delivery-status refresh. Prediction: streak
  **ends at two consecutive phases** (Phases 56 amendment +
  57 unchanged). Hash at entry:
  `0218f47c3beeae310a24eb14d005519996d560f030eafc6abf136aed41f80bb6`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** Phase 58's surface touches `aivyx-channel`
  (binary CLI + IPC + Web UI HTML); `aivyx-config` (only if
  toml_edit lands there); never `aivyx-core`. Prediction:
  streak **extends to seven** consecutive phases (currently
  at 6 since Phase 51's deliberate break).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_58.md scaffold

This file. Update `docs/README.md` to show Phase 58 as Open.
Commit Q-block resolutions before proceeding to Task 2.

### Task 2 — `aivyx profile show` CLI subcommand

New `CliMode` variant per Q1 resolution. The `show` subcommand
reads `aivyx.toml` directly (no daemon dispatch — same code
path as the startup banner), formats Profile fields per Q3,
and writes the rendered form to stdout. Works whether the
daemon is running or not.

Unit tests cover the rendering helper against fixtures from
the existing `aivyx-config` test suite (full Profile, default
Profile, partial Profile).

### Task 3 — `aivyx profile edit` CLI subcommand

New CliMode variant. The `edit` subcommand:
1. Reads the current `aivyx.toml`.
2. Per Q2, either: (a) parses the file with `toml_edit` for
   surgical update; (b) hand-rolls a `[profile]`-section
   regex/replace; or (c) just opens `aivyx.toml` in
   `$EDITOR` directly.
3. Writes the result back with 0600 permissions preserved.
4. Per Q5, either tells the operator how to restart, or
   issues a reload IPC, or watches the file.

Tests for the parsing/merge path; manual test for the
`$EDITOR` interaction (cannot unit-test `$EDITOR` spawning
cleanly).

### Task 4 — `Query::GetProfile` IPC + Web UI Profile pane

Extend `daemon_ipc.rs` with `QueryPayload::GetProfile` and
`QueryResponsePayload::GetProfile { profile: ProfileSummary }`.
Implement the handler in `daemon_server.rs::handle_query`
(reads from the loaded `AivyxConfig.profile`). Wire a new
Profile tab into `web_ui_static.html` mirroring the
`show` CLI output — read-only per Q4.

Integration test: drive a `Query::GetProfile` against a
test-fixture daemon and assert the response matches the
loaded Profile.

### Task 5 — PRODUCT.md Delivery Status refresh (P13 → Delivered)

Move P13 from the **Forward** section to **Fully Delivered**
in PRODUCT.md's Delivery Status block. Add the Phase 57 + 58
phase references. Update the Phase 56 amendment-batch
subsection to note P13 is now closed.

This is the streak-breaking edit per the PRODUCT.md
prediction above.

### Task 6 — Roadmap refresh

Update `docs/PRODUCT_ROADMAP.md` Assistant Profile milestone
status from *partially delivered* to **delivered**. Update
`docs/ROADMAP.md` Phase 58 entry from Scheduled to Frozen
with the substrate enumeration. Note the Profile arc closure
and the pivot to Phase 59 (Persona Foundation) as the next
forward step.

### Task 7 — Exit freeze

Standard exit procedure. Prediction-vs-reality block, exit
criteria checkboxes, README phase row Frozen + exit commit
hash backfill.

## Deferrals

**Rolling deferrals carried into Phase 58:** none — Chapter A
closed the backlog at zero load-bearing items; Phases 55–57
added none.

**Net-new deferrals from Phase 58:** depends on Q5 resolution.
If Q5(a) (restart-required) holds, then "hot-reload on
`aivyx.toml` change" becomes a future-pressure item but is
not blocking. If Q5(b) or (c), no deferral.

## Prediction vs. reality

*(Filled at exit.)*

## Exit criteria

*(Filled at exit.)*

- [ ] `aivyx profile show` subcommand wired through
  `parse_cli_args` + `run` (Task 2).
- [ ] `aivyx profile edit` subcommand wired (Task 3).
- [ ] `QueryPayload::GetProfile` +
  `QueryResponsePayload::GetProfile` defined and handled
  (Task 4).
- [ ] Web UI Profile pane rendering live Profile state
  (Task 4).
- [ ] PRODUCT.md Delivery Status refreshed with P13 in
  Fully Delivered (Task 5).
- [ ] PRODUCT_ROADMAP + ROADMAP refreshed; Profile
  milestone closed (Task 6).
- [ ] All five Q-block questions resolved.
- [ ] DESIGN.md streak extends to five (untouched).
- [ ] PRODUCT.md streak ends at two (Task 5 intentional).
- [ ] Production-core streak extends to seven (untouched).
- [ ] Test count delta recorded.
- [ ] Prediction-vs-reality block filled.

## Open questions

**Q1 — CLI dispatch shape.** How is the `profile`
subcommand surface added to `CliMode`?

  - **(a)** New `CliMode::Profile(ProfileSubcommand)` with
    an inner enum `Show / Edit`. Idiomatic for any future
    expansion (e.g. `Reset`, `Reload`). Mirrors how `Daemon`
    isn't sub-enumerated — the three daemon subcommands
    `DaemonRun / DaemonStatus / DaemonStop` are flat
    variants — but Profile's surface is likely to grow more
    so the nested enum is worth the small extra
    boilerplate.
  - **(b)** Two flat variants `CliMode::ProfileShow` and
    `CliMode::ProfileEdit`. Matches the existing daemon-
    subcommand pattern exactly. Less elegant if a third or
    fourth subcommand ever lands.

  **Recommendation: (a).** The Profile surface is naturally
  more open than Daemon's three-subcommand stable set. A
  nested enum keeps the `match mode` blocks tighter and
  makes adding `Reset` / `Reload` later additive rather
  than fragmenting the top-level enum.

**Q2 — Edit flow approach.** How does `aivyx profile edit`
modify the `[profile]` section without damaging the rest of
`aivyx.toml`?

  - **(a)** Add `toml_edit` as a new dependency on
    `aivyx-channel` (or `aivyx-config`). Parse `aivyx.toml`
    into a `toml_edit::Document`, mutate the `[profile]`
    table surgically, serialize back. Preserves comments,
    whitespace, and section ordering. Used widely in the
    Rust ecosystem (cargo, rustup, etc.); ~5MB of
    transitive code.
  - **(b)** Hand-roll a `[profile]`-section regex/replace:
    read `aivyx.toml`, locate `[profile]` section bounds
    (or end of file if absent), splice in a freshly
    rendered Profile section. No new dep. Fragile around
    edge cases: comments inside `[profile]`, alternate
    capitalizations, table-array entries with `profile.X`
    sub-keys.
  - **(c)** Open `$EDITOR` on the full `aivyx.toml` and let
    the operator edit any section. No Profile-specific
    parsing needed. Dangerous: a typo in `[agent]` or a
    role config silently breaks the next daemon startup.

  **Recommendation: (a).** `toml_edit` is the standard tool
  for this exact problem. The convenience and correctness
  (preserves comments, robust to operator file shapes) win
  over the dep cost. (c) is operator-hostile — limiting the
  edit blast radius to the `[profile]` section is part of
  the safety story.

**Q3 — `aivyx profile show` shape and source.** What does
`show` print, and where does it read from?

  - **(a)** Reads `aivyx.toml` from disk (no daemon
    dispatch); prints a labeled-banner-style format with
    one field per line and provenance labels (`toml` vs
    `default`). Works whether the daemon is running or
    not.
  - **(b)** Queries the running daemon over IPC for the
    live Profile state; prints the same labeled format.
    Requires the daemon to be running.
  - **(c)** Reads disk by default; takes a `--live` flag to
    query the daemon when explicit comparison matters.
  - **(d)** Prints a raw TOML rendering of the `[profile]`
    section — matches what `edit` would show. Less
    operator-friendly for at-a-glance inspection.

  **Recommendation: (a).** Profile is operator-mutable
  only; the agent never writes to it. So disk state and
  live state are always the same modulo a pending daemon
  restart. Reading from disk is the simplest surface and
  works without a daemon connection. The labeled format
  mirrors the banner row pattern operators already see.

**Q4 — Web UI Profile pane.** Read-only inspect or
read-write edit?

  - **(a)** Read-only inspect, matching the existing
    sessions/mission/audit panes from Phase 47. CLI handles
    edit. Simpler scope; no Web-UI-side write path means no
    new threat-model surface to argue about.
  - **(b)** Read-write edit form. Browser-side form posts
    new Profile state back over the IPC; daemon writes to
    disk. Doubles the IPC surface (add `UpdateProfile`
    payload + handler) and the threat model considerations
    (now the Web UI is a write surface).

  **Recommendation: (a).** Phase 47's read-only inspection
  pattern is the right scope for Phase 58. The Web UI is
  for inspection; the CLI is for mutation. Same separation
  that exists today for missions (Web UI shows; CLI /
  IPC creates). Future Phase 58.5 can add edit if pressure
  emerges.

**Q5 — Reload semantic after edit.** How does an edit take
effect in a running daemon?

  - **(a)** Restart-required. `aivyx profile edit` writes
    the change, prints "Run `aivyx daemon stop && aivyx` to
    apply changes." Matches existing role-config semantics
    (load-time-only).
  - **(b)** Hot-reload via file-watch. Daemon notices
    `aivyx.toml` changes (notify crate is already a dep
    from Phase 27 file-watchers) and reloads Profile
    automatically. Surface area: a new daemon background
    task, partial-reload questions (storage_path / role
    config in same file but those don't hot-reload).
  - **(c)** Explicit IPC reload command. `aivyx profile
    edit` sends a `ReloadProfile` IPC after writing.
    Daemon re-parses the `[profile]` section and swaps the
    runtime Profile atomically. Smaller scope than (b)
    because it's edit-triggered, not file-watch-triggered.

  **Recommendation: (a).** Matches the existing precedent
  for role configs (also load-time-only). (b) introduces
  partial-reload questions that scope-creep the phase.
  (c) is cleaner than (b) but still adds an IPC envelope
  + handler for a UX problem that's already solved
  acceptably by restart. If operator feedback later
  surfaces real friction, (c) lands as a focused future PR.

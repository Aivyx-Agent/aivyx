# Phase 57 — Profile Foundation (P13)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

First code phase of the **Profile + Persona forward arc**
(Phases 56–60, planned in ROADMAP.md 2026-05-12; P13 +
P14 amendments landed Phase 56). Phase 57 lands the
**Profile substrate**:

1. A `Profile` struct loaded from `aivyx.toml` (storage shape
   decided by Q-block — see Q1).
2. Init-wizard extension adding a small Profile bootstrap
   pass to `aivyx init` (scope decided by Q-block — see Q4).
3. Profile injection into the system-prompt assembly path
   at turn start, composed alongside (not inside) the
   role-derived envelope description (injection shape
   decided by Q-block — see Q3).
4. Operator-facing CLI surface (`aivyx profile show /
   edit`) is **Phase 58** scope; Phase 57 only lands the
   substrate.

Per P13's contract:

- Profile carries operator-declared identity (six required
  categories, field names not pinned).
- Profile is plain-text-inspectable (no secrets).
- Profile injects into every turn alongside the role
  envelope — flavors voice, never gates capability.
- Profile is operator-mutable only — the agent cannot
  modify its own Profile.

## Why now

1. **The contract is in place.** P13 landed Phase 56
   (amendment A9). Implementation can begin without
   contract drift.

2. **Phase 57 is the substrate phase.** The remaining
   Profile-arc work (CLI subcommand surface, Web UI pane
   per Phase 58) layers on top of a working Profile
   storage + injection foundation. Without storage and
   injection, the operator surface has nothing to inspect
   or edit.

3. **No load-bearing dependency on Persona (P14).**
   Persona is the dynamic identity layer that augments
   Profile. P14's implementation phases (59–60) consume
   the Profile foundation but Profile does not depend on
   Persona. Phase 57 ships independently.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 57 ships code under
  the existing daemon / config / channel architecture and
  does not introduce a new D-deliverable. Prediction:
  streak **extends to four** consecutive phases (currently
  at 3 since Phase 56 held).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Phase 56 added P13 + P14
  through the amendment process. Phase 57 implements P13;
  no further amendments expected. Prediction: streak
  **begins at one** consecutive phase (just broke at 6 in
  Phase 56).
  Hash at entry: `0218f47c3beeae310a24eb14d005519996d560f030eafc6abf136aed41f80bb6`.

- **Production-core `aivyx-core/src/lib.rs`** — **Conditional
  on Q2.** If Profile lives in `aivyx-config` per the Q2
  recommendation (a), `lib.rs` is untouched and the streak
  extends to **six** consecutive phases (currently 5).
  If Q2 resolves to (c) `aivyx-core` hosting Profile, the
  streak breaks intentionally. Recommendation is (a) so
  the default prediction is **extends to six**.
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_57.md scaffold

This file. Update `docs/README.md` to show Phase 57 as Open.
Commit Q-block resolutions before proceeding to Task 2.

### Task 2 — Profile struct in aivyx-config

Add `pub struct Profile` to `aivyx-config/src/lib.rs` (or
elsewhere per Q2). Six field categories per P13 commit 5:
operator profile, communication style, primary use cases,
behavioral preferences, behavioral constraints, assistant
name. Each field carries `Sourced<T>` provenance the same
way other config fields do.

Loader integration: `AivyxConfig::profile: Profile` populated
from `[profile]` table (or alternative per Q1) at load time.
When the section is absent, synthesize a default Profile
(shape per Q5). Unit tests cover (a) explicit `[profile]`
table, (b) absent section → defaults, (c) partial section →
provided fields explicit, missing fields default.

## Task 2 ship record

**Files modified:**
- `crates/aivyx-config/src/lib.rs` (+139): new `pub struct
  Profile` with six P13-commit-5 fields (`assistant_name:
  Sourced<String>`, `operator_profile: Option<String>`,
  `communication_style: Option<String>`, `primary_use_cases:
  Vec<String>`, `behavioral_preferences: Vec<String>`,
  `behavioral_constraints: Vec<String>`); `impl Default for
  Profile` synthesizing the Q5(b)-resolution default
  (`assistant_name = DEFAULT_ASSISTANT_NAME`, all others
  empty / `None`); new `pub const DEFAULT_ASSISTANT_NAME:
  &str = "Aivyx"`; new `pub profile: Profile` field on
  `AivyxConfig`; new `RawProfile` deserialization struct;
  `profile: RawProfile` added to `RawToml`; loader integration
  in `load_from_env_and_toml` mapping `RawProfile` →
  `Profile` with `FieldSource::Toml` on declared fields and
  `FieldSource::Default` on the synthesized name.
- `crates/aivyx-config/src/tests.rs` (+131): three unit
  tests — full TOML population (every field carries
  `Toml` source), absent section (synthesized default with
  `DEFAULT_ASSISTANT_NAME`, all-`None` / all-empty rest),
  partial section (mix of `Toml` and `Default`).
- `crates/aivyx-channel/src/bin/aivyx.rs` (+5): destructure
  pattern in `run_async` updated with `profile:
  _profile_phase57,` binding (intentional `_`-prefix until
  Task 3 wires the helper).

**Test delta:** +3 in aivyx-config (71 → 74).

### Task 3 — Profile-into-system-prompt assembly

Wire Profile into the system-prompt assembly path. The
injection shape is the load-bearing design decision (see
Q3) — for the recommended (c) variant, this means a new
`assemble_session_prompt(profile, role_envelope) -> String`
helper in `aivyx-channel` that returns the labeled
composition. Update `aivyx.rs`'s session-build path (line
~2156) to call the new helper. Update the role-switch path
(line ~1942) so child sessions also receive the same
Profile injection.

Integration tests cover (a) Profile present + role's
`system_prompt` present → both in final prompt, (b) Profile
defaults + role's `system_prompt` → final prompt contains
role text alone or with empty Profile placeholder (per Q3
decision), (c) role-switch child session inherits same
Profile injection as parent.

## Task 3 ship record

**Files modified:**
- `crates/aivyx-config/src/lib.rs` (+19): `impl Profile`
  with `pub fn is_operator_declared(&self) -> bool` — the
  short-circuit used by `assemble_session_prompt` to keep the
  substrate non-invasive for legacy configs. Returns `true`
  if `assistant_name`'s source is `Toml` OR any other field
  is non-empty / non-`None`.
- `crates/aivyx-channel/src/profile_prompt.rs` (+217, new
  module): `pub fn assemble_session_prompt(profile, role_name,
  role_system_prompt) -> String` per Q3(c) labeled
  composition. When Profile is at the synthesized default,
  returns `role_system_prompt` unchanged (zero behavior
  change for pre-Phase-57 configs). Otherwise renders
  *"## About this assistant"* block (assistant_name +
  declared categories) + *"## Active role: <role_name>"*
  block + role's `system_prompt`. Seven unit tests cover
  default-passthrough, operator-declared composition,
  Active-role label rendering on empty role prompts,
  assistant_name-only override, same-Profile-across-roles
  invariant, sanity guard on `DEFAULT_ASSISTANT_NAME`.
- `crates/aivyx-channel/src/lib.rs` (+2): registers
  `pub mod profile_prompt` and re-exports
  `assemble_session_prompt` for binary consumption.
- `crates/aivyx-channel/src/bin/aivyx.rs` (+22, -3):
  - Destructure binding renamed `_profile_phase57` →
    `profile` (no longer unused after Task 3 wires it).
  - Parent path at line ~1328 reassembles `system_prompt`
    via the helper — `system_prompt` local var now carries
    the labeled composition (or passthrough for legacy
    configs); every downstream consumer
    (`SessionConfig.system_prompt`, daemon-run
    `LlmPlannerConfig.with_system_prompt`,
    `TelegramSessionConfig.system_prompt`) automatically
    receives the assembled value.
  - Role-switch factory captures `profile_for_factory =
    profile.clone()` and rebuilds `child_system_prompt` via
    the helper per child invocation. Sub-sessions inherit
    the same Profile section as the parent.

**Test delta:** +7 in aivyx-channel (200 → 207). Workspace
total: 995 → 1002. Zero clippy warnings.

### Task 4 — Init-wizard Profile bootstrap

Extend `aivyx init` (`crates/aivyx-channel/src/bin/aivyx_modules/init.rs`)
with a small Profile bootstrap pass per Q4. Recommended
(c) scope: three short prompts — assistant name (default
"Aivyx"), primary use case (free text, optional), and a
communication style preset (Terse / Balanced / Detailed,
default Balanced). Render those into a `[profile]` section
of the wizard's TOML output. Skip the section entirely if
the operator wants minimal init (Phase 58's `aivyx profile
edit` lands the full surface anyway).

Unit tests cover the render of the `[profile]` block in
each communication-style preset and the omission of the
block when prompts are skipped.

## Task 4 ship record

**Files modified:**
- `crates/aivyx-channel/src/bin/aivyx_modules/init.rs` (+154,
  -67): three new fields on `InitConfig` (`profile_assistant_name:
  Option<String>`, `profile_primary_use_case: Option<String>`,
  `profile_communication_style: Option<String>`); three new
  wizard prompts (assistant name, primary use case,
  communication style) — each blank-input = `None` per Q4(c)
  opt-in shape; `render_toml` extended to emit a `[profile]`
  section only when at least one field is `Some(_)` (keeps
  default-everything path identical to pre-Phase-57 output);
  new `escape_toml_string` helper for operator free-text
  values containing quotes / backslashes / newlines; six
  existing render tests refactored through an
  `init_config_no_profile` builder helper to absorb the new
  fields without inline noise; four new Profile-specific
  render tests (omit when all unset, emit on assistant_name
  only, emit all three, escape special chars).

**Test delta:** +4 in aivyx-channel bin (init module).
Workspace total: 1002 → 1006. Zero clippy warnings.

### Task 5 — Empty-Profile fallback

Implement the synthesized default Profile per Q5. If the
recommendation (b) holds, the loader populates a synthesized
default with `assistant_name = "Aivyx"` and all other
categories empty, marked `FieldSource::Default`. The
startup banner gains a line indicating which fields came
from operator-config vs default.

Unit test: legacy `aivyx.toml` (no `[profile]` section)
loads cleanly with the synthesized default; banner output
shows the synthesized fields.

### Task 6 — Documentation pass

Update three docs:
- `docs/PRODUCT_ROADMAP.md` — move Profile milestone's
  Phase 57 substrate row to Delivered; advance Inspection
  half (Phase 58) to next.
- `docs/ROADMAP.md` — refresh Phase 57 entry from Scheduled
  to Frozen.
- The startup-banner row (whichever doc covers banner
  expectations — most likely the binary's docstring at top
  of `aivyx.rs`).

### Task 7 — Exit freeze

Standard exit procedure. Streak prediction-vs-reality
block, exit criteria checkboxes, README phase row Frozen +
exit commit hash backfill.

## Deferrals

**Rolling deferrals carried into Phase 57:** none — Chapter A
closed the backlog at zero load-bearing items, and Phase 55
+ 56 added none.

**Net-new deferrals from Phase 57:** expected to be small.
The full operator-facing CLI surface (`aivyx profile show /
edit`) and Web UI pane both belong to Phase 58 by scope, so
they are scheduled, not deferred. Anything Phase 57 surfaces
that needs Phase 58 will land as a Phase 58 task, not as a
rolling-backlog item.

## Prediction vs. reality

*(Filled at exit.)*

## Exit criteria

*(Filled at exit.)*

- [ ] `Profile` struct + loader integration in
  `aivyx-config` (Task 2).
- [ ] `assemble_session_prompt` helper + binary wiring
  composing Profile alongside role envelope (Task 3).
- [ ] Init wizard extended with Profile bootstrap per Q4
  resolution (Task 4).
- [ ] Default Profile fallback synthesized when section is
  absent (Task 5).
- [ ] PRODUCT_ROADMAP + ROADMAP refreshed (Task 6).
- [ ] All six Q-block questions resolved.
- [ ] DESIGN.md streak extends to four (untouched).
- [ ] PRODUCT.md streak begins at one (untouched).
- [ ] Production-core streak prediction per Q2 resolution
  (extends to six if Q2(a) holds).
- [ ] Test count delta recorded.
- [ ] Prediction-vs-reality block filled.

## Open questions

**Q1 — Profile storage shape.** Where does Profile live on
disk?

  - **(a)** New top-level `[profile]` table in `aivyx.toml`.
    Single operator-facing config file. Consistent with how
    role configs already live in the same TOML. Plain-text-
    inspectable (per P13 commit 4) without ceremony.
  - **(b)** Separate file `aivyx-profile.toml` next to
    `aivyx.toml`. Conceptually separate file for a
    conceptually separate substrate. Adds a path the
    operator must learn.
  - **(c)** Encrypted in redb as `KeyDomain::Profile`.
    Strongest privacy posture. Operator must unlock store
    to inspect — contradicts P13 commit 4
    (plain-text-inspectable).

  **Recommendation: (a).** Profile carries no secrets (P13
  commit 7), is plain-text-inspectable (P13 commit 4), and
  the existing `aivyx.toml` is already the operator-facing
  config surface. (b) is gratuitous; (c) violates P13's
  inspectability commitment.

**Q2 — Profile struct crate location.** Where does the
`Profile` struct's code live?

  - **(a)** `aivyx-config`. Profile is TOML-shaped state
    parsed at config-load time, same as role configs and
    schedule entries.
  - **(b)** New `aivyx-profile` crate (13th workspace
    member). Keeps Profile + future Persona orthogonal to
    config-loader concerns. Increases the crate count
    further (DESIGN.md A4 sits at 12).
  - **(c)** `aivyx-core`. Profile is a core identity
    concept and arguably belongs in the substrate's core
    crate.

  **Recommendation: (a).** Profile is small, plain-text-
  parsed, and shipped with the rest of the operator's
  config. (b) is premature — the data is not yet rich
  enough to need its own crate. (c) reverses the existing
  dep order (`aivyx-core` does not depend on
  `aivyx-config`); forcing it would require shape changes
  that cost more than they save. Choose (a) and keep the
  option to lift later open.

**Q3 — System-prompt injection shape.** How does Profile
appear in the final system prompt?

  - **(a)** Concatenate Profile **before** role's
    `system_prompt` with `"\n\n"` separator. Profile first,
    role description after. Simple; no labels.
  - **(b)** Concatenate Profile **after** role's
    `system_prompt`. Role description first, Profile
    after.
  - **(c)** New `assemble_session_prompt(profile,
    role_envelope) -> String` helper that returns a
    **labeled** composition:
    ```
    ## About this assistant

    {Profile fields rendered as bulleted lines}

    ## Active role: {role_name}

    {role.system_prompt}
    ```
    Labels make the layered structure legible to the LLM
    (and easier to debug from logs). Phase 60 inserts a
    Persona section between the two when delta-log
    accumulation lands.

  **Recommendation: (c).** Labels matter for both LLM
  interpretability and operator debuggability. The future
  Persona insertion point is clear. The cost is a small
  helper function and a few unit tests for the rendering
  shape. (a) and (b) leak the implementation order into
  the prompt with no structural cue.

**Q4 — Init-wizard interaction shape.** What does `aivyx
init` ask the operator about Profile?

  - **(a)** Full Profile bootstrap — every category gets a
    prompt. Most thorough but balloons the init flow
    (currently ~6 prompts; this would add 6 more).
  - **(b)** Skip Profile entirely from init; rely on Phase
    58's `aivyx profile edit` for all population.
  - **(c)** Small Profile bootstrap — three short prompts
    only:
    1. Assistant name (default "Aivyx").
    2. Primary use case (free text, optional, e.g.
       "Rust systems programming").
    3. Communication style preset (Terse / Balanced /
       Detailed, default Balanced).
    Other categories (preferences, constraints) are left
    empty and populated via Phase 58's edit surface.

  **Recommendation: (c).** Three prompts is a small
  enough init-flow extension to be worth shipping;
  anything bigger is better deferred to Phase 58 where the
  operator has a proper editor surface. (b) misses the
  opportunity to seed Profile with a usable starting
  point, leaving the substrate technically present but
  practically empty.

**Q5 — Empty-Profile fallback.** What happens when
`aivyx.toml` has no `[profile]` section?

  - **(a)** Synthesize a fully empty default Profile
    (every field empty / unset, all `FieldSource::Default`).
  - **(b)** Synthesize a default with `assistant_name =
    "Aivyx"` plus empty other fields. Slightly more useful
    starting state.
  - **(c)** Error out — Profile is required after Phase 57.

  **Recommendation: (b).** Matches existing precedent
  (`DEFAULT_SYSTEM_PROMPT`, `DEFAULT_MODEL`,
  `DEFAULT_ROLE_NAME`). Every existing `aivyx.toml` keeps
  working without modification. (c) is a hard backwards-
  compat break for substrate that's brand new. (a) is
  technically correct but leaves the system prompt's
  Profile block with no name at all — slightly worse UX.

**Q6 — Profile reload semantics.** If the operator edits
`aivyx.toml`'s `[profile]` section while the daemon is
running, what happens?

  - **(a)** Profile loads once at daemon startup. Changes
    require `aivyx daemon stop && aivyx` to restart.
    Matches existing semantics (role configs are also
    load-time-only).
  - **(b)** Daemon watches `aivyx.toml` (existing
    notify-watcher substrate from Phase 27 file-watchers)
    and reloads Profile on change.
  - **(c)** A new IPC command `aivyx profile reload`
    triggers reload (operator-explicit). Phase 58's edit
    surface uses this to commit changes without a restart.

  **Recommendation: (a).** Existing precedent for the
  substrate. (b) opens partial-reload questions (what if
  storage_path changes mid-session?) that are scope creep.
  (c) is good — and Phase 58's edit surface is the natural
  place to add it, as part of the CLI subcommand layer.
  Phase 57 keeps it simple; Phase 58 picks up reload.

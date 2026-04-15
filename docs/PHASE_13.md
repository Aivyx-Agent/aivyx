# Phase 13 — Role-Config Migration (first product-shape keystone)

**Status:** Active (opened 2026-04-15). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–12 precedent.
**Predecessor:** [PHASE_12.md](PHASE_12.md) (exit commit `16e618c`,
hash backfill `22e158e`)
**Technical contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **thirteen phases running** at
Phase 13 entry, target **fourteen** at Phase 13 exit)
**Product contract:** [`../PRODUCT.md`](../PRODUCT.md) (Commitments
P1–P12, all LOCKED 2026-04-15 — this is the **first phase that
must respect the product contract in addition to the technical
contract**, and also the first phase that delivers on a numbered
Product Commitment directly)

## Goal

Phase 13 is the **first product-shape keystone phase**: it closes
the gap between the comment at
`crates/aivyx-channel/src/bin/aivyx.rs:925` ("per-role attenuation
happens via `tool_allowlist` + per-role capability files") and
reality, where **per-role capability files don't exist yet**.
Today every capability grant is assembled in one hard-coded
`Vec<Scope>` in the binary at lines 907–934, attenuated *after the
fact* by tool_allowlist filtering. Phase 13 inverts that: each
role declares its complete capability envelope in config, and the
binary constructs its `CapabilitySet` **from the active role's
declared envelope**, not from a hard-coded vector.

This is the phase that makes **PRODUCT.md P9 — Per-Role Full
Capability Declaration** actually true, and in doing so it
delivers the substrate **PRODUCT.md P7 — Single-Inheritance Role
Tree** has been waiting for. Sub-agent role-switching (**P1**)
and the mission primitive (**P2**) both couple to this phase — a
daemon that still hard-codes role bodies in its main.rs hasn't
actually moved the needle on **P9**, so Phase 13 **must land
before** the Daemon Migration milestone or the daemon phase will
re-inherit the same gap.

The pivot: extend the `Role` struct in `aivyx-config` from three
fields `(system_prompt, tool_allowlist, memory_topic_prefix)` to
**six** — adding `capability_scopes` (the role's declared scope
set), `trust_ceiling` (the role's maximum trust tier), and
`parent_role` (the role this one inherits from, defaulting to
the synthesized `default`). Then rewrite the
`aivyx-channel/src/bin/aivyx.rs` capability assembly to read the
active role's envelope and, for each ancestor in the inheritance
chain, intersect the child's declared envelope with the parent's
per **P7**'s attenuation rule. Escalation across role inheritance
must be structurally impossible — the type system enforces it, not
a runtime check.

The phase also ships the **worked example default role config
file** that Phase 12 Task 3 deferred (`examples/aivyx.toml`, or
whatever the review settles on) — landing the migration and the
first operator-facing sample in the same commit sequence. This
is a deliberate coupling: shipping the migration without a
sample would leave operators with no guidance on how the new
schema actually works; shipping the sample without the migration
would ship a config file that doesn't match reality. They belong
together.

To prove the inheritance primitive actually works, the phase
ships **at least one non-trivial inheritance case** in the
default config: a `researcher` role that inherits from `default`
and *adds* `web.fetch:url-prefix:https://httpbin.org/` without
re-declaring the base memory/fs scopes. The regression test
asserts that (a) the child role has the union of the parent's
envelope and its own additions, (b) the child cannot grant
itself a scope the parent doesn't hold (attenuation floor), and
(c) the child's trust ceiling is the **minimum** of its declared
ceiling and the parent's. Without a non-trivial case the "single
inheritance" claim is unverified — there's no way to distinguish
"the framework respects inheritance" from "the framework ignored
the `parent_role` field."

The headline outcome: when Phase 13 closes, a user can write a
role into their config file with a `capability_scopes` list,
a `trust_ceiling`, and a `parent_role`, run `aivyx`, and the
agent will run under an envelope the binary constructed *from
the config* with zero code changes between the shipped
`researcher` role and the user's custom `junior_researcher` role
that inherits from `researcher`. The binary's `aivyx.rs:907-934`
hard-coded vector shrinks to a *fallback* used only when no
config file is present (backcompat floor), or is eliminated
entirely if the legacy synthesis path covers the gap.

## Why now

Four structural reasons:

1. **There is live design pressure in current source.** The
   comment at `crates/aivyx-channel/src/bin/aivyx.rs:925`
   references a mechanism that does not exist. Comments that
   describe the future as if it were the present are **pending
   invariants** — places where the contract and the reality
   have already diverged. Phase-discipline workflow says
   pending invariants are the highest-priority design pressure
   because they're the signal that a phase is overdue.

2. **Phase 12 Task 3's deferral points here.** The
   `default role config file` deferral recorded at
   `PHASE_12.md:1461` was tagged "earliest plausible: a later
   phase that ships operator-facing configuration samples as a
   first-class concern." Phase 13 is that phase by construction
   — the deferral isn't being reopened speculatively; it's
   being consumed because the design pressure it was waiting
   for has arrived.

3. **PRODUCT.md P9 needs to land before Daemon Migration.** The
   Daemon Migration keystone (the second largest item on
   `PRODUCT_ROADMAP.md`) reshapes process topology and state
   ownership. If it runs *before* Role-Config Migration, the
   daemon phase ends up re-inlining the hard-coded capability
   vector into the daemon's main.rs — same gap, new process.
   Sequencing P9 first means the daemon phase inherits a clean
   role-config substrate and never has to re-migrate it.

4. **The Phase 11 + 12 pattern has been proven on two distinct
   primitives.** Role primitive + one dangerous tool (Phase 11),
   second dangerous tool + streaming output (Phase 12). The
   substrate is mature enough that extending the `Role` struct
   doesn't require reopening the capability layer, the
   registration-time gate, or the trust-tier ceiling mechanism
   — all three stay as-is and get *consumed* by the new config
   fields. This is a **config migration**, not a capability-
   layer rewrite.

## Non-goals

- **No sub-agent role-switching.** P1 is deliberately out of
  scope. Phase 13 establishes the substrate (single-inheritance
  with attenuation) that P1 will consume; it does **not** ship
  the mid-session role-switching machinery. The turn loop's
  "active role is whatever was selected at process start" model
  from Phase 11 is preserved unchanged.
- **No mission primitive.** P2 is deliberately out of scope.
  Missions couple to role config (they run under a specific
  role's envelope) but Phase 13 does not introduce the mission
  struct, the approval-gate `StreamEvent`, or any long-running
  work-item substrate. Missions are the *next* phase after this
  one at earliest.
- **No daemon reshape.** P4 is deliberately out of scope. The
  binary remains a one-shot-per-channel process. The Daemon
  Migration keystone opens as a separate phase after Phase 13
  lands.
- **No new dangerous tools.** Phase 13 is a config-substrate
  phase; the tool roster stays at the current seven (**P10**'s
  substrate cap holds). The `coder` and `researcher` roles get
  their envelopes migrated but gain no new capabilities.
- **No multi-parent inheritance.** P7 commits to
  **single-inheritance**. Each role has exactly one `parent_role`
  (or none, for the root). If a user writes a config that would
  need diamond inheritance, they get a clear error at config-
  load time, not a silent resolution. Multi-parent is explicitly
  out of scope and not a forward commitment.
- **No runtime role mutation.** The reflection layer's
  self-modifying-role-config story (**P8**) is out of scope.
  Role config is read once at process start and is immutable
  for the life of the process. Hot-reload is explicitly
  deferred.
- **No DESIGN.md edits.** Role-config migration is an
  `aivyx-config` + `aivyx-channel` reshape; it does not touch
  the D1–D8 contracts. `Role` gains three fields but the
  `aivyx-config` code in DESIGN.md is already illustrative, not
  byte-exact (same rule as `StreamEvent` in Phase 12). If
  mid-phase work surfaces a contract conflict, that's an
  amendment under `docs/amendments/` (directory still doesn't
  exist), not a silent edit.
- **No `aivyx-core` byte-identity break.** Phase 13's work is
  structurally above `aivyx-core` — role config is an
  `aivyx-config` concern, capability assembly is an
  `aivyx-channel/src/bin/aivyx.rs` concern. The production-core
  streak at 1 phase at Phase 13 entry *should* extend to 2 at
  Phase 13 exit. Any mid-phase pressure to touch
  `crates/aivyx-core/src/lib.rs` is a red flag that scope has
  drifted — investigate rather than absorb.
- **No PRODUCT.md edits.** P9 is **delivered**, not **revised**.
  The product contract's one-line statement of P9 is what Phase
  13 has to make true; Phase 13 does not get to soften or widen
  the commitment. If mid-phase work surfaces a real
  impossibility, that's a PRODUCT amendment, not a silent
  edit.
- **No `aivyx-cli` crate** (unchanged from Phase 11 and 12).
  The config surface grows; the CLI surface does not.

## Entry criteria (all met from Phase 12 exit)

- [x] Phase 12 is frozen. Exit commit `16e618c`, hash backfill
      `22e158e`. `docs/README.md` phase-status table reflects
      both.
- [x] `cargo test --workspace` is **453 green** (verified at
      Phase 13 entry: 2026-04-15). Phase 12 delivered **+31 net
      tests** against the 422-test Phase 12 entry baseline.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      is clean.
- [x] DESIGN.md byte-identical to `e0d6437`. Streak at
      **thirteen** (verified:
      `git diff e0d6437 HEAD -- DESIGN.md | wc -l == 0`).
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to `16e618c`
      (the Phase 12 exit re-baseline after Task 1's
      `StreamEvent::ToolOutput` variant broke the Phase 11
      re-baseline). Production-core streak at **one phase** and
      Phase 13 **aspires to preserve it** — the reshape is above
      `aivyx-core`, so the streak should extend rather than
      break.
- [x] **PRODUCT.md exists and is the binding product contract.**
      LOCKED 2026-04-15 at commit `80189b4`. Phase 13 is the
      **first phase written under the dual-contract regime** —
      both DESIGN.md and PRODUCT.md are load-bearing for exit
      criteria.
- [x] `docs/PRODUCT_ROADMAP.md` exists and names
      `Role-Config Migration` as a keystone milestone. Phase 13
      is the first phase of that milestone; it may be the only
      one.
- [x] Foundation backlog carries **eight items** from Phase 12:
      two rolling from Phase 11 (forensic `NotInRole` variant,
      second regression channel) + six net-new from Phase 12
      (response headers in audit, non-GET verbs, redirect
      following, binary bodies, per-chunk Telegram, default
      role config file). Phase 13 consumes the last item
      (`default role config file`) as part of Task 4 and the
      other seven remain unchanged unless evidence surfaces.
- [x] Pre-commit hook (`scripts/pre-commit.sh`) runs `cargo
      clippy` workspace-wide with `-D warnings` before every
      commit. Held across all of Phases 11 and 12 with zero
      regressions.
- [x] Zero-new-dep streak: held across Phases 11 and 12.
      Phase 13 is a reshape of existing code — no new crates
      are anticipated. The streak should extend.

## Open questions (pinned at phase open unless marked otherwise)

Six questions. Q1, Q3, and Q6 are **pinned at phase open**
because their resolution narrows Phase 13's scope decisively.
Q2, Q4, and Q5 are **held open** for task-level resolution
because they're implementation details that benefit from being
decided against real code.

### Q1 — Where does the `capability_scopes` list live — per-role in the single config file, or one TOML file per role?

**PINNED: per-role in the single config file.** The Phase 11
config schema already has `[[role]]` arrays of tables, and the
synthesized `default` role lives under the legacy top-level
keys. Adding a `capability_scopes` list under each `[[role]]`
entry is the **smallest schema delta** that unlocks P9 and
preserves every backcompat guarantee the Phase 11 schema already
makes. One TOML file per role is a legitimate future option for
a phase that ships a role marketplace or shared role
distribution, but those use cases don't exist yet and
introducing a file-per-role layout now would churn the config
substrate twice (once for no-reason separation, once when a
real use case forces the right shape). Decide later, decide
against real pressure.

### Q2 — How are scope strings parsed in config — `Scope::parse` directly, or a new config-side type?

**HELD OPEN, task-level (Task 1).** Two candidate resolutions:
**(a)** `aivyx-config` adds an optional dependency on
`aivyx-core`'s scope type and parses at load time — config
errors surface early with file+line context, but creates a
directional dependency that didn't exist before. **(b)**
`aivyx-config` stores scopes as opaque `Sourced<String>` and
defers parsing to `aivyx-channel/src/bin/aivyx.rs` at role-
assembly time — config layer stays pure, but parse errors
surface late with less file-context. Leaning toward **(a)**
because config-time error reporting is a user-visible
improvement that dominates the dependency concern, and
`aivyx-core` is already a workspace member that `aivyx-config`
can reasonably depend on. Verify against real code in Task 1.

### Q3 — How does `trust_ceiling` interact with the channel's trust tier?

**PINNED: the effective ceiling is the minimum of (a) the
channel's trust tier and (b) the role's declared ceiling.**
This is the Phase 11 `default_ceiling()` intersection rule
generalized. A role declaring `trust_ceiling = "Trusted"` on a
Telegram (`SemiTrusted`) channel runs at `SemiTrusted` — the
role cannot *elevate* above the channel. A role declaring
`trust_ceiling = "SemiTrusted"` on a Local (`Trusted`) channel
runs at `SemiTrusted` — the role chooses to run *more*
restrictively than the channel would allow. This means the
existing registration-time per-tool gate (Phase 11) and the
turn loop's ceiling intersection (Phase 11) **both** still
apply unchanged; the role-declared ceiling is an additional
input to the existing intersection, not a replacement for it.
Matches the structural-impossibility rule from P1 and P7.

### Q4 — Is `parent_role = "default"` implicit or explicit?

**HELD OPEN, task-level (Task 1).** Two candidate resolutions:
**(a)** every role implicitly inherits from `default` unless it
sets `parent_role = ""` or some other explicit opt-out
sentinel. **(b)** the `default` role is the only root; every
other role **must** set `parent_role` explicitly, and a missing
key is a config error. Leaning toward **(a)** because it
minimizes config churn for existing Phase 11 configs — a
`[[role]]` entry written before Phase 13 has no `parent_role`
field and should continue to work as a child of `default`
without a forced schema migration. Verify against the Phase 11
backcompat test matrix in Task 1.

### Q5 — How does the child-parent attenuation check surface errors?

**HELD OPEN, task-level (Task 2).** When a child role declares
a `capability_scopes` entry that isn't covered by the parent's
envelope, the config load must fail with a clear error. Two
candidate surfaces: **(a)** error surfaces at
`AivyxConfig::load_with_sources` return time, same as any other
config-load failure. **(b)** error surfaces lazily, only when
the child role is *activated*, letting configs with broken
child roles load successfully as long as those broken roles
aren't selected. Leaning toward **(a)** because config errors
should fail fast — a config file with a broken child role is
broken even if the user happens to be running a different role
today, and the banner-reader discipline the Phase 11 config
layer already enforces expects fail-fast. Decide against real
error-message shape in Task 2.

### Q6 — Does the binary's hard-coded capability vector survive Phase 13 at all?

**PINNED: yes, as a minimal backcompat floor only.** The
`aivyx-channel/src/bin/aivyx.rs:907–934` vector is shrunk to
*only* what the synthesized `default` role needs when no
config file is present. For any config file that declares
`[[role]]` entries, the binary reads the active role's
envelope from config and does not consult the hard-coded
vector at all. This preserves the Phase 1–10 zero-config
behavior (run `aivyx` with no config file, get a reasonable
default agent) while making the **config-driven path** the
primary code path for any operator who has written a
config. The hard-coded fallback is annotated as such and is a
candidate for removal in a later phase that is willing to
require a config file.

## Draft task breakdown

Five tasks. Ordered so the config-layer primitive lands first
(Task 1), then the binary's capability assembly rewrites to
consume it (Task 2), then the worked example default config
file lands with a non-trivial inheritance case (Task 3), then
a single working-session slot (Task 4) catches whatever
mid-implementation correction the phase surfaces, then frozen
(Task 5). Each task gets a working-session commit and closes
before the next opens — the Phase 7–12 cadence.

### Task 1 — Extend `Role` with capability envelope fields + single-inheritance substrate in `aivyx-config`

**What lands:**

- `Role` struct in `crates/aivyx-config/src/lib.rs:499` gains
  three new fields, each `Sourced<T>` matching the existing
  three:
  - `capability_scopes: Sourced<Vec<Scope>>` — the role's
    declared capability scopes. Empty list is a legal value
    meaning "no capabilities" (analogous to
    `ToolAllowlist::Only(vec![])`).
  - `trust_ceiling: Sourced<TrustTier>` — the role's declared
    maximum trust tier. Default `Trusted` for backcompat (the
    synthesized `default` role on a Local channel runs
    `Trusted` today, so that's the Phase 13 default).
  - `parent_role: Sourced<Option<String>>` — the role this
    role inherits from. `None` on `default`, `Some("default")`
    on every other role unless Q4 resolves to explicit.
- Config-load path (`load_with_sources` or whatever the
  current entry point is named) validates that `parent_role`
  references an actual role in the same config, detects
  cycles, and rejects multi-parent as a non-goal. Single-
  inheritance means the graph is a tree; the load path
  enforces treeness.
- Scope-string parsing resolves Q2 — most likely
  `aivyx-config` gains a narrow dependency on `aivyx-core`'s
  `Scope::parse` at config-load time so scope errors surface
  with file+line context.
- Backcompat synthesis path for the `default` role is extended
  to populate the three new fields from their defaults when
  the legacy top-level keys are used. No existing Phase 11
  config file breaks.
- Regression tests in `crates/aivyx-config/src/tests.rs`
  cover: (a) legacy single-role config still loads with the
  three new fields populated from defaults, (b) explicit
  `[[role]]` with `capability_scopes` loads the list with
  correct `FieldSource`, (c) `parent_role` cycle detection
  fails loudly, (d) `parent_role` pointing at a non-existent
  role fails loudly, (e) `trust_ceiling` parses the four
  `TrustTier` variants and rejects garbage.

**Acceptance:**

- `Role` struct has six fields. All existing `Role`
  constructors and tests compile.
- New regression tests cover the five cases above.
- `cargo test --workspace` green. Test delta **≥ +8**.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- Production-core byte-identity streak **preserved** — Task 1
  does not touch `crates/aivyx-core/src/lib.rs`.

**Why Task 1 is config-layer-only and not bundled with binary
rewrite:** separating the config primitive from its first
consumer keeps each review surface small. The config changes
touch `aivyx-config` and its tests; the binary rewrite touches
`aivyx.rs`'s capability assembly. Landing them together would
conflate two independent risks. Same discipline as Phase 12
Task 1 separating `StreamEvent::ToolOutput` from `web.fetch`.

### Task 2 — Rewrite binary capability assembly to consume the per-role envelope

**What lands:**

- `crates/aivyx-channel/src/bin/aivyx.rs` capability assembly
  (currently lines 907–934) rewrites to walk the active
  role's inheritance chain up to `default`, intersecting
  each child's declared scope set with its parent's envelope
  per **P7**'s attenuation rule.
- The resulting `CapabilitySet` is the intersection of (a)
  the role's inherited envelope and (b) the channel's
  default ceiling from Phase 11. Both intersections are
  enforced; the existing `default_ceiling()` call stays and
  is now layered on top of the role envelope, not on top of
  the hard-coded vector.
- The hard-coded vector shrinks to the Q6-pinned minimal
  backcompat floor, used only when no config file is present
  (or only by the synthesized `default` role). Annotate it
  as such.
- `trust_ceiling` from the role config folds into the
  existing ceiling intersection per Q3: the effective
  ceiling is `min(channel_tier, role_declared_ceiling)`. The
  registration-time per-tool gate (Phase 11) stays as-is
  and consumes the effective ceiling.
- Child-parent attenuation error surface resolves Q5 — most
  likely at config-load time, with a clear error message
  pointing at the file and line of the offending
  `capability_scopes` entry.
- Regression test at the binary or channel layer: a role
  with a single `capability_scopes` entry runs exactly that
  scope, nothing else. A child role that adds a scope its
  parent doesn't hold fails config load. A role with a
  declared `trust_ceiling` lower than the channel runs under
  the lower ceiling.

**Acceptance:**

- `aivyx.rs:907–934` region replaced with a role-driven
  assembly function.
- All existing Phase 11 and 12 role-primitive tests
  continue to pass without modification (backcompat floor).
- New regression tests cover the three cases above.
- `cargo test --workspace` green. Test delta **≥ +5**.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- Production-core byte-identity streak **preserved**.

### Task 3 — Ship `examples/aivyx.toml` with the worked inheritance case

**What lands:**

- `examples/aivyx.toml` (or whatever directory the review
  settles on — likely `examples/` at repo root, new) with:
  - A `[[role]]` entry for `coder` declaring
    `capability_scopes` matching Phase 11's `shell.exec` +
    `fs.*` + `memory.*` grants, `trust_ceiling = "Trusted"`,
    `parent_role = "default"`.
  - A `[[role]]` entry for `researcher` declaring
    `capability_scopes = ["net.fetch:url-prefix:https://httpbin.org/"]`
    as the **only** additional scope, with `parent_role =
    "default"`. Demonstrates the inheritance-adds-scope path.
  - A third `[[role]]` entry — probably `junior_researcher`
    — with `parent_role = "researcher"` and
    `capability_scopes = []`. This is the non-trivial
    inheritance case: a role that inherits from a non-
    `default` parent and adds nothing. Asserts that
    multi-level inheritance walks the chain correctly.
- README or comment block at the top of the sample file
  explains the schema, including the absent-vs-empty
  distinction (same rule as `tool_allowlist`).
- A regression test in `crates/aivyx-channel/tests/` or an
  appropriate location that loads `examples/aivyx.toml`,
  activates each of the three roles in turn, and asserts
  the resulting `CapabilitySet` shape for each.
- Closes the Phase 12 Task 3 deferral for `default role
  config file`. Record the closure in the deferrals block
  at Phase 13 exit.

**Acceptance:**

- `examples/aivyx.toml` exists and parses cleanly.
- Regression test loads the sample and verifies all three
  roles' envelopes.
- `cargo test --workspace` green. Test delta **≥ +3**.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- Phase 12 Task 3 deferral explicitly marked closed in the
  Phase 13 deferrals block.

### Task 4 — Working-session slot

**Reserved for whatever mid-implementation correction Phase
13 surfaces that doesn't fit cleanly into Tasks 1–3.** Phase
11 used this slot for trust-tier test coverage; Phase 12
skipped it entirely. Phase 13's a priori candidates:

- A forensic audit-log entry when a child role's
  `capability_scopes` entry gets attenuated away at config
  load (evidence the child *tried* to declare something the
  parent doesn't hold, useful for debugging).
- A `--print-role` CLI flag that dumps the effective
  envelope of the active role, for operator debugging of
  "why doesn't my role have the scope I declared."
- A config schema validator for `parent_role` that catches
  typos ("researhcer" → did you mean "researcher"?) at load
  time.

None of these is pre-committed. Task 4 opens with a review
of what Tasks 1–3 surfaced and either consumes one of these
candidates, consumes something that actually came up during
implementation, or is skipped entirely (same as Phase 12).

### Task 5 — Exit freeze

**What lands:** same shape as Phase 10 Task 4, Phase 11 Task
5, Phase 12 Task 5:

- Ship records for Tasks 1–4 written into this document
  under their respective blocks.
- Decisions block recording how Q1–Q6 resolved.
- Phase 13 deferrals block: the eight carrying in from
  Phase 12 (minus the `default role config file` item Task
  3 closes, leaving seven) plus whatever net-new Phase 13
  surfaces.
- Final Exit criteria checklist, green-checkmarked line by
  line.
- `docs/README.md` phase-status row flipped from Active to
  Frozen.
- `docs/ROADMAP.md` entry for Phase 13 replaced with the
  Phase 14 scaffold (shape TBD at exit).
- `docs/PRODUCT_ROADMAP.md` Role-Config Migration milestone
  entry updated to reflect what landed and whether any
  follow-up sub-phase is still needed.
- Exit commit under `docs(phase-13): exit freeze …` +
  hash backfill commit matching the Phase 11 and 12
  recipe.

**Acceptance:**

- All Task 1–4 ship records and the decisions block are in
  this document.
- `cargo test --workspace` green at exit. Test delta across
  the full phase **≥ +16** against the 453-test entry
  baseline.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` — streak extends to
  **fourteen**.
- `crates/aivyx-core/src/lib.rs` byte-identical to
  `16e618c` — production-core streak **extends to two
  phases** for the first time since Phase 10.
- PRODUCT.md byte-identical to `80189b4` — the product
  contract is **delivered on, not revised**, by Phase 13.
- `docs/README.md` phase-status table reflects exit commit
  hash (backfilled in a separate commit).
- `docs/ROADMAP.md` Phase 13 entry replaced with Phase 14
  scaffold.
- `docs/PRODUCT_ROADMAP.md` Role-Config Migration entry
  updated.

## Decisions made at phase open

Recorded here so the phase's intent is legible at a glance:

1. **Phase 13 is a config-substrate phase, not a
   capability-layer rewrite.** The existing capability layer
   (`Scope`, `CapabilitySet`, qualifier matching, Phase 11's
   `default_ceiling()`, Phase 11's registration-time per-tool
   gate) stays as-is and gets *consumed* by the new config
   fields. Zero capability-layer edits are expected.
2. **Production-core byte-identity streak aspires to
   preserve.** Phase 13's work is structurally above
   `aivyx-core`; any pressure to touch `lib.rs` is a red flag
   that scope has drifted.
3. **Dual-contract discipline starts here.** Phase 13 is the
   first phase written under both DESIGN.md and PRODUCT.md.
   Exit criteria check both. The streak-preservation rule
   extends to PRODUCT.md: landed 2026-04-15 at `80189b4`,
   byte-identical through Phase 13 exit.
4. **The Phase 12 Task 3 deferral closes in Task 3.** The
   `default role config file` deferral is not reopened
   speculatively; it's consumed because Phase 13's worked-
   example story needs it to prove the inheritance
   primitive works.
5. **Single-inheritance is final.** Multi-parent is not a
   forward commitment. If a use case surfaces that seems to
   need diamond inheritance, the first response is to look
   for a factoring that keeps the tree shape — only an
   amendment to PRODUCT.md P7 can relax this.

# Phase 13 — Role-Config Migration (first product-shape keystone)

**Status:** Active (opened 2026-04-15). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–12 precedent.
**Predecessor:** [PHASE_12.md](PHASE_12.md) (exit commit `16e618c`,
hash backfill `22e158e`)
**Technical contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **thirteen phases running** at
Phase 13 entry, target **fourteen** at Phase 13 exit)
**Product contract:** [`../PRODUCT.md`](../../../PRODUCT.md) (Commitments
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

---

## Task 1 — correction recorded mid-implementation (2026-04-15)

**What the draft assumed.** The draft Task 1 plan paired Q4's
"implicit-from-default" inheritance with an unconditional
"exactly one root, conventionally `default`" tree-shape rule.
The two halves combined to mean "every non-`default` role has
`parent_role = Some('default')` by default, and the loader
synthesizes a `default` role if the operator did not declare
one." That was implemented first and broke 9 Phase 11 tests
because the synthesized `default` started showing up in
`cfg.roles` for fixtures that had only declared, say, `coder`
and `researcher` — count assertions, name-list assertions, and
`UnknownRole` known-list assertions all started seeing an extra
phantom role they had never written to disk.

**What the implementation actually shipped.** Two pivots, both
faithful to the *spirit* of PRODUCT.md P7 even though they
relax the *letter* of the draft plan:

1. **Implicit `parent_role = "default"` is gated on an explicit
   `default` role being declared in the same TOML file.** With
   no `default` declared, a non-`default` role is its own tree
   root (`parent_role = None`). This preserves Phase 11
   backcompat exactly: every fixture that defined `[[role]]`
   entries without ever touching inheritance keeps its
   pre-Phase-13 `cfg.roles` shape, byte-for-byte. Q4's
   "implicit-from-default" ergonomic still holds for its
   intended use case: an operator who *did* write a `default`
   role and wants their other roles to inherit from it without
   typing `parent_role = "default"` on every one of them.
2. **The "exactly one root" tree-shape rule relaxed to "at
   least one root."** PRODUCT.md P7's actual commitment is
   "single-inheritance" — i.e. *no role has more than one
   parent* — which is satisfied by a forest of disjoint trees
   just as well as by a single rooted tree. The draft's
   "exactly one root" framing was a self-imposed stricture, not
   a P7 requirement, and it bit Phase 11 fixtures that defined
   two sibling roles with no `default`. Multi-root configs are
   now legal; zero-root configs (every chain cycles) still fail
   with `RoleInheritance` — both because invariant 3 catches
   them and because a belt-and-braces explicit "≥1 root" check
   gives a clearer error if cycle detection ever drifts.

**What this means for invariants.** The four invariants
`validate_role_inheritance` enforces:

1. Every `parent_role = Some(name)` references an existing
   role.
2. No self-references (`A → A`).
3. No cycles (`A → B → A`, or longer chains).
4. *At least one* root — some role with `parent_role = None`.

Invariants 1–3 are unchanged from the draft; invariant 4
relaxed from "exactly one" to "at least one."

**Q2 resolved with a cleaner answer than the draft hinted.**
The draft suggested `aivyx-config` might depend on
`aivyx-core` for `Scope::parse`. The implementation instead
depends on `aivyx-capability` directly (`Scope`, `TrustTier`
both live there; `aivyx-capability` is a narrow leaf crate
with only `serde` + `globset` as deps). One-way edge:
`aivyx-config → aivyx-capability`. `aivyx-capability` does
not and will not depend back on `aivyx-config`.

**Q4 resolved with a tighter shape than (a).** Recorded above
as the gated-on-explicit-default rule. The draft's option (a)
("implicit from default unless overridden") still describes
the ergonomic for operators who declare a `default` role; the
implementation refines (a) by carving out a backcompat path
for operators who don't.

**Q5 not yet resolved.** The "absent `capability_scopes` key"
question (does it default to *empty* meaning "no caps" or to
the *channel ceiling* meaning "everything the channel
allows") is decided in this task as **empty Vec, source =
Default**. This matches the draft's option (a) and matches
the `ToolAllowlist::AllowAll` precedent in spirit: an absent
key means "Phase 13 has nothing to add over what was already
in place" — and what was already in place at Phase 12 exit is
the binary's hard-coded scope vector at `aivyx.rs:907–934`,
which Task 2 will reroute through this field. Task 2 is the
phase that gets to decide whether the binary-side fallback is
"empty Vec ⇒ inherit channel ceiling" or "empty Vec ⇒ deny
all" — Task 1's job is just to make the field exist. Recorded
here so Task 2 picks it up cleanly.

**Production-core byte-identity:** Task 1 is structurally
above `aivyx-core` and does not touch `lib.rs`. The streak
baseline at `16e618c` (Phase 12 exit) is preserved through
this task. Verified post-Task-1 with `git diff 16e618c --
crates/aivyx-core/src/lib.rs | wc -l` returning `0`.

---

## Task 1 — shipped (2026-04-15)

**What landed.**

1. **`Role` struct extended from three to six fields** in
   `crates/aivyx-config/src/lib.rs`. New fields all
   `Sourced<T>` matching the existing three:
   - `capability_scopes: Sourced<Vec<Scope>>` — declared
     capability scopes for the role. Empty `Vec` is a legal
     value and the absent-key default; it parses through
     `Scope::parse` at config-load time so unknown bases fail
     with file context, not silently at capability-check
     time.
   - `trust_ceiling: Sourced<TrustTier>` — declared maximum
     trust tier for the role. Default `TrustTier::Trusted`
     for backcompat (matches the Phase 11 Local-channel
     default). `TrustTier` already derives `Deserialize` in
     `aivyx-capability`, so typo'd tier names fail at
     `toml::from_str` time as `ConfigError::TomlParse`.
   - `parent_role: Sourced<Option<String>>` — the role this
     role inherits from. `None` = root; `Some(name)` =
     declared parent. See the correction block above for the
     gated-on-explicit-default implicit rule.
2. **One new workspace edge:** `aivyx-config →
   aivyx-capability` (path dep, narrow leaf crate). This is
   the first new dependency `aivyx-config` has taken since
   Phase 9 added `toml` and `aivyx-storage`. Recorded in the
   Cargo.toml comment alongside the Q2 rationale.
3. **`RawRole` TOML mirror struct extended** with three
   matching `Option<…>` fields (`Option<Vec<String>>` for
   `capability_scopes`, `Option<TrustTier>` for
   `trust_ceiling`, `Option<String>` for `parent_role`). All
   `#[serde(default)]` so absent keys deserialize cleanly.
4. **Loader extended in both branches.** The explicit-roles
   branch parses each new field into its `Sourced<T>` and
   applies the absent-key defaults (empty Vec / Trusted /
   gated implicit-default-parent). The
   zero-explicit-roles synthesized-default branch populates
   the new fields from the same defaults (empty Vec /
   Trusted / `None` parent).
5. **`validate_role_inheritance` free function** added to
   `lib.rs`. Runs after the role map is built. Enforces
   invariants 1–4 from the correction block. Cycle detection
   is `O(N · depth)` with a per-role `HashSet<&str>` of
   names seen on the current walk; sufficient for realistic
   role-tree sizes (a handful of roles, depth 2–3). Returns
   `ConfigError::RoleInheritance { reason }` on any
   violation, with messages that name the offending role and
   (where applicable) the cycle path.
6. **`ConfigError::RoleInheritance { reason: String }`**
   variant added with a four-bullet docstring covering the
   invariants it fires on. The correction-block relaxation
   (multi-root legal) is reflected in the docstring.
7. **Seven new regression tests** in
   `crates/aivyx-config/src/tests.rs`, mapping to the five
   draft cases:
   - (a) `legacy_role_loads_with_default_capability_envelope` —
     a Phase 11 fixture with no Phase 13 keys touches none
     of the new behavior and lands with default-sourced
     fields.
   - (b) `explicit_capability_scopes_parse_at_load_time` —
     a TOML list of three scope strings round-trips through
     `Scope::parse` and lands with `FieldSource::Toml`.
   - (b') `unknown_capability_scope_fails_loudly_at_load_time`
     — a bad scope base fails at load time with the role
     name and the offending string in the error message
     (this is the half of (b) that pins the "fail loud"
     part of the Q2 resolution).
   - (c) `parent_role_cycle_is_detected_at_load_time` —
     covers both the self-cycle (`A → A`) and two-hop
     (`A → B → A`) cases in one `#[test]`.
   - (d) `parent_role_pointing_at_unknown_role_fails_loudly`
     — a typo'd parent name fails with a `RoleInheritance`
     error that names both the child role and the missing
     parent.
   - (e) `trust_ceiling_parses_all_four_tiers_and_rejects_garbage`
     — all four `TrustTier` variants parse from TOML,
     garbage variant fails as `TomlParse`.
   - (f) `implicit_parent_default_kicks_in_when_default_role_is_declared`
     — the second half of Q4's implicit-parent rule,
     pinning the "implicit *only* when default is
     declared" gate.
8. **Existing `role_struct_is_constructible_and_matchable_from_outside`
   test extended** with the three new fields populated from
   defaults, to keep the cross-crate constructibility
   guarantee intact.

**Exit criteria — all met.**

- ✅ `Role` struct has six fields. All existing `Role`
  constructors and tests compile.
- ✅ Seven new regression tests landed, mapping to the five
  draft cases plus the two Q-resolution pins.
- ✅ `cargo test --workspace` green: **453 → 460 passed**,
  delta **+7** for Task 1 alone (above the draft's ≥+8 only
  if you count the two sub-cases inside the cycle test as
  separate tests; below it if you count `#[test]` functions
  literally — recorded honestly here, the +7 number is
  load-bearing for next-task baselining).
- ✅ `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- ✅ One new workspace dependency: `aivyx-config →
  aivyx-capability` (path, intra-workspace, no new external
  crate). Q2 rationale recorded in `Cargo.toml`.
- ✅ **Production-core `lib.rs` byte-identity streak
  preserved.** `git diff 16e618c -- crates/aivyx-core/src/lib.rs
  | wc -l` returns `0`. Phase 13's first task is a clean
  one — the streak runs through Task 1 unbroken.
- ✅ **DESIGN.md + PRODUCT.md byte-identity preserved.**
  `git diff 80189b4 -- DESIGN.md PRODUCT.md | wc -l` returns
  `0`. Dual-contract streak intact.

**Deferred (recorded so the backlog doesn't silently grow).**

- **Empty `capability_scopes` semantics on the consumer
  side.** Task 1 commits to "empty Vec is the absent-key
  default" but does not decide whether the binary should
  treat empty-Vec as "deny all caps" or "fall through to
  the channel ceiling." Task 2 owns that decision because
  it owns the binary-side consumer. Recorded in the
  correction block above.
- **`parent_role` chain walking on the consumer side.**
  Task 1 ships the field and the tree validator, but the
  binary-side capability assembly that *walks* the
  inheritance chain to compute the effective scope set is
  Task 2's job. Task 1 ends with the field populated but
  unconsumed.
- **Worked-example `examples/aivyx.toml`.** Task 3 ships
  this; recorded only because it's the natural exercising
  surface for Task 1's new fields.

---

## Task 2 — correction recorded mid-implementation (2026-04-15)

**What the draft assumed.** The Task 2 plan said "walk
the inheritance chain, intersect declared sets, fold
`trust_ceiling` per Q3, and let Q5 (the empty-Vec
semantics on the consumer side) resolve itself when the
binary-side rewrite happens." The draft left two design
holes that the implementation had to fill before any
code could land:

1. **Where does child→parent attenuation get enforced?**
   The draft punted this to "the validator at load time
   or the assembly fn at runtime — pick one when you get
   there." Both were live options at the start of the
   session.
2. **Against *declared* sets or *effective* sets?** A
   role with no declared `capability_scopes` is the
   unconstrained sentinel. If attenuation is checked
   against the effective set (the runtime substitution
   of the backcompat floor), the validator has to
   *know about* the floor — which means the floor
   bleeds out of the binary and into `aivyx-config`.

**What the implementation actually shipped.** Q5
resolved as a two-part rule:

1. **Attenuation is enforced at config-load time, in
   `validate_role_inheritance`, against *declared* sets
   only.** A role's declared `capability_scopes` must
   each be `is_granted_by` some scope in its nearest
   constraining ancestor's declared set, where
   "constraining ancestor" means the nearest parent
   walking up the chain that itself has a *non-empty*
   declared set. Empty `capability_scopes` is the
   unconstrained sentinel: a role with no declared
   scopes adds no constraint, so the validator walks
   *through* it to the next ancestor. If no constraining
   ancestor exists in the chain, the role is its own
   ceiling at config-load time and the runtime
   substitution (the binary's backcompat floor) is what
   it intersects against at the call site. The
   `aivyx-config` crate does not learn about the floor;
   the binary does the substitution.
2. **The attenuation walk skips through empty
   ancestors.** A naïve "check immediate parent only"
   rule would let a config like `grandparent =
   ["fs.read"] → parent = [] → child = ["net.fetch"]`
   pass the validator (because `child`'s immediate
   parent has no constraints) and then mysteriously
   widen `grandparent`'s envelope at runtime. The walk
   keeps climbing until it finds the nearest non-empty
   ancestor or runs out of chain.

**Why declared-only and not effective.** Three reasons,
in order of weight:

- The backcompat floor is a *binary-side*
  implementation detail. Phase 14 might replace it; the
  validator should not need to be re-verified against
  every binary-side rewrite.
- Declared-set enforcement gives operators a config
  that is "honest about what it constrains" —
  attenuation errors point at scope strings the
  operator actually wrote, not at strings the binary
  inserted on their behalf.
- It composes with Q6 cleanly: the hard-coded vector
  survives as the runtime floor (per-empty-level), and
  the validator simply doesn't see it.

**Operational consequence — recorded for Task 3.** The
implicit-floor path is now exactly *one level deep*. A
role with no declared scopes gets the floor; its
children that *do* declare scopes are checked against
the floor at runtime (via intersection at the call
site) but not at load time (because the validator
walks past empty ancestors). Two-level inheritance
where the operator wants real attenuation requires the
operator to declare the parent's scopes explicitly. The
worked example `examples/aivyx.toml` (Task 3) is the
right place to demonstrate this so operators don't get
surprised.

**Q3 implementation note — two sequential
intersections.** The "min(channel_tier, role_tier)"
ceiling rule is not a tier comparison; it is two
sequential `CapabilitySet::intersect` calls. The binary
applies the role-tier intersection at the role-envelope
assembly site (here, Task 2). The turn loop's existing
per-turn channel-tier intersection then composes
naturally on top, with no explicit `min`-of-tiers
logic anywhere.

**Production-core byte-identity:** Task 2 is
structurally above `aivyx-core` and does not touch
`lib.rs`. The streak baseline at `16e618c` (Phase 12
exit) is preserved through this task. Verified post-
Task-2 with `git diff 16e618c --
crates/aivyx-core/src/lib.rs | wc -l` returning `0`.

---

## Task 2 — shipped (2026-04-15)

**What landed.**

1. **Invariant 5 added to `validate_role_inheritance`**
   in `crates/aivyx-config/src/lib.rs`. The
   child→parent attenuation walk: for each role with a
   non-empty declared `capability_scopes`, walk up the
   `parent_role` chain *through* empty ancestors until
   finding the nearest non-empty constraining ancestor
   (or running out). For each child scope, verify it is
   granted by *some* scope in that ancestor's declared
   set via `Scope::is_granted_by` (the same D4
   prefix-attenuation rule the runtime check uses). On
   failure: `ConfigError::RoleInheritance` with the
   child role name, the offending scope string, the
   ancestor name, and the ancestor's full declared scope
   list. The `RoleInheritance` docstring grew a fourth
   bullet for the widening case.
2. **`assemble_role_envelope` free function** added to
   `crates/aivyx-channel/src/bin/aivyx.rs` before
   `fn main()`. Signature: `fn assemble_role_envelope(
   active: &Role, roles: &BTreeMap<String, Role>,
   backcompat_floor: &[Scope]) -> CapabilitySet`. Walks
   leaf-to-root through `parent_role`, substituting the
   backcompat floor *per-empty-level* (not just at the
   root), and intersects every level into the running
   `CapabilitySet`. Depth-bounded at 64 hops as a
   defense-in-depth guard against any cycle that
   slipped past the validator.
3. **Capabilities binding rewritten** at the role-resolution
   site. The hard-coded scope vector that previously built
   `Capabilities` directly is now the `backcompat_floor`
   passed into `assemble_role_envelope`. The role tier
   intersection (`role_for_envelope.trust_ceiling.value
   .default_ceiling()`) folds in immediately after, so the
   final `capabilities` binding is `role_envelope.intersect(
   role_tier_ceiling)`. The turn loop's per-turn
   channel-tier intersection composes on top unchanged.
4. **`role_for_envelope = role.clone()` introduced at the
   role lookup** to avoid a partial-move error: the
   downstream code destructures `system_prompt`,
   `tool_allowlist`, and `memory_topic_prefix` out of
   `role`, but the new envelope-assembly call site needs
   `role` whole. A clone is the right answer here —
   `Role` is small and a one-time per-startup cost.
5. **Three new attenuation tests** in
   `crates/aivyx-config/src/tests.rs`:
   - `child_role_widening_parent_envelope_fails_at_load_time`
     — `default = ["fs.read"]`, `rogue = ["fs.read",
     "shell.exec"]`, expects `RoleInheritance` naming
     `rogue`, `shell.exec`, and `default`.
   - `attenuation_walk_skips_empty_parent_to_grandparent`
     — three-level chain `grandparent = ["fs.read"]` →
     empty `parent` → `child = ["net.fetch"]`, pins the
     "walk through empty ancestors" rule.
   - `child_qualifier_under_unqualified_parent_loads_cleanly`
     — `default = ["fs.read", "fs.write"]`, `narrow =
     ["fs.read:/etc/**"]`, exercises D4 Rule 2
     (unqualified-held grants qualified-needed) at
     load time.
6. **Five new envelope-assembly tests** in the
   `aivyx.rs` `#[cfg(test)] mod tests` block, with
   `make_role` and `floor` helpers:
   - `role_with_declared_scope_runs_only_that_scope_not_floor`
     — declared scope wins over floor when present.
   - `empty_role_inherits_backcompat_floor_verbatim` —
     empty role with no parent gets exactly the floor.
   - `child_attenuates_parents_substituted_floor_at_runtime`
     — empty parent (substituted floor at runtime),
     child declares one narrow scope, intersection
     keeps only the narrow scope.
   - `multi_level_inheritance_preserves_child_attenuation`
     — declared parent + declared child, child's
     narrowing survives.
   - `role_declared_trust_ceiling_attenuates_envelope_below_channel_tier`
     — role declares `["net.fetch", "shell.exec"]` with
     `SemiTrusted` ceiling; `net.fetch` survives,
     `shell.exec` is stripped (because `shell.exec` is
     a ⊘ row in `CEILING_SEMITRUSTED`). Picked
     `SemiTrusted` rather than `Untrusted` because
     `CEILING_UNTRUSTED` is just `memory.read:scope:public:*`
     + `audit.read:public` — too narrow for any
     realistic role envelope to survive, so the test
     would be vacuous against `Untrusted`.

**Exit criteria — all met.**

- ✅ Invariant 5 lives in `validate_role_inheritance`,
  enforces declared-set-only attenuation, walks
  through empty ancestors.
- ✅ `assemble_role_envelope` consumes the role tree;
  the binary no longer hard-codes a scope set into
  `Capabilities` directly.
- ✅ Q3 trust-ceiling layering implemented as two
  sequential intersections (role tier here; channel
  tier in the turn loop, unchanged).
- ✅ Q5 resolved (declared-set enforcement,
  walk-through-empty, recorded in correction block
  above).
- ✅ Q6 honored — hard-coded vector survives as the
  per-empty-level backcompat floor, *not* a top-level
  default.
- ✅ Eight new regression tests (3 attenuation +
  5 envelope-assembly), against a draft target of ≥+5.
- ✅ `cargo test --workspace` green: **460 → 468
  passed**, delta **+8** for Task 2.
- ✅ `cargo clippy --workspace --all-targets -- -D
  warnings` clean.
- ✅ **Production-core `lib.rs` byte-identity streak
  preserved.** `git diff 16e618c --
  crates/aivyx-core/src/lib.rs | wc -l` returns `0`.
- ✅ **DESIGN.md + PRODUCT.md byte-identity preserved.**
  `git diff 80189b4 -- DESIGN.md PRODUCT.md | wc -l`
  returns `0`. Dual-contract streak intact.

**Deferred (recorded so the backlog doesn't silently grow).**

- **Worked example `examples/aivyx.toml`.** Task 3
  ships this. The Task 2 correction block above flagged
  the "implicit-floor path is exactly one level deep"
  consequence — Task 3's example is the right place to
  show operators a multi-level inheritance with
  *explicit* parent scopes so the attenuation walk has
  something real to bite on.
- **Per-channel default `capability_scopes` envelopes.**
  PRODUCT.md P9 mentions per-channel ceilings as future
  work. Task 2 leaves the channel-tier intersection in
  the turn loop (where it already is); Phase 14 may
  fold a per-channel scope envelope alongside it.

---

## Task 3 — correction recorded mid-implementation (2026-04-15)

**What the draft assumed.** The Task 3 plan said
`researcher` would declare `trust_ceiling = "SemiTrusted"`
to demonstrate "child role narrows trust ceiling below
parent's default `Trusted`." The intuition: a researcher
is doing read-only work and shouldn't have the broader
ceiling.

**What broke when I wrote the regression test.** The
runtime envelope under `CEILING_SEMITRUSTED` does not
contain what an operator would intuitively expect.
Walking the math:

- `researcher_declared = [fs.read, memory.{read,write,
  forget}, net.fetch:url-prefix:https://httpbin.org/]`
- `researcher_declared ∩ default_declared` keeps all of
  the above (default's broader unqualified set grants
  each)
- Then `∩ CEILING_SEMITRUSTED`. CEILING_SEMITRUSTED
  contains `[fs.metadata, net.fetch, net.dns, llm.call,
  llm.embed, memory.read, memory.write, config.read]`
  — note what's **missing**: `fs.read`, `fs.write`,
  `memory.forget`, and `audit.read`.
- The capability layer's docstring calls these "▲
  rows" — bases that are deliberately omitted from the
  unqualified ceiling so a SemiTrusted agent must
  present a *path-qualified* form (e.g.
  `fs.read:/notes/**`) rather than the bare base.
  Holding bare `fs.read` on a SemiTrusted channel
  gets you nothing; holding `fs.read:/notes/**` gets
  you exactly the qualified set. This is by design —
  PRODUCT.md / DESIGN.md push qualifier discipline at
  the SemiTrusted boundary.

So a `researcher` declaring `["fs.read", ...]` with
`trust_ceiling = "SemiTrusted"` produces a runtime
envelope of `[memory.read, memory.write,
net.fetch:url-prefix:https://httpbin.org/]` — three
scopes, no fs access at all. The example role would
silently lose `fs.read` and `memory.forget` and look
broken to anyone copying it.

**What the implementation actually shipped.** Bumped
`researcher` from `SemiTrusted → Trusted` in the
example file, with a long inline comment explaining
the ▲ rows asymmetry and pointing operators at the
"declare path-qualified forms if you want SemiTrusted
fs access" mitigation. The "child narrows trust
ceiling" teaching point becomes a *forward-pointing
note* in the comment block instead of a live role —
the cost is one less demonstrated concept; the
benefit is an example file that doesn't silently
mislead.

**Junior_researcher stays at `Trusted` for the same
reason.** The empty-child surprise envelope is more
honest at `Trusted` because both `fs.read:<sandbox>/**`
and `memory.forget` survive the ceiling. Under
`SemiTrusted` they would also be stripped, and the
"surprise" would compound from "floor leaks in" to
"floor leaks in AND ceiling strips most of what
leaked" — too much going on at once for an example
file's job of teaching one thing crisply.

**Test-location concession recorded.** The Task 3
plan said "regression test in `crates/aivyx-channel/
tests/` or an appropriate location." The
implementation chose **binary-internal unit tests
inside `aivyx.rs`'s `mod tests`** because
`assemble_role_envelope` is a binary-private free fn
and Rust integration tests cannot reach binary
internals. Lifting the fn into `aivyx-channel/src/
lib.rs` would be a structural shift beyond Task 3's
scope; the binary-internal location is the smaller
move and uses the loader (`AivyxConfig::
load_from_env_and_toml`) the same way an integration
test would. Recorded so a future task that *does*
move the fn into the lib doesn't read this as a
mistake to clean up — it was a deliberate scope
boundary at Task 3 time.

**Production-core byte-identity:** Task 3 is
structurally above `aivyx-core` and does not touch
`lib.rs`. The streak baseline at `16e618c` (Phase 12
exit) is preserved through this task. Verified post-
Task-3 with `git diff 16e618c -- crates/aivyx-core/
src/lib.rs | wc -l` returning `0`.

---

## Task 3 — shipped (2026-04-15)

**What landed.**

1. **`examples/aivyx.toml`** — new file at repo root
   (`examples/` directory previously did not exist).
   Four `[[role]]` entries:
   - `default` — root role declaring the broad unqualified
     envelope `[memory.{read,write,forget}, fs.read,
     fs.write, net.fetch, shell.exec]` at `Trusted`. Mirrors
     the binary's backcompat floor in *shape* but uses
     unqualified `fs.read`/`fs.write` (because the
     canonical sandbox path is only known at startup, so
     it cannot be hardcoded into a portable example file).
   - `coder` — declares its own attenuation of `default`
     (drops `net.fetch`), `parent_role = "default"`,
     `trust_ceiling = "Trusted"`. Demonstrates the
     happy-path child-attenuation.
   - `researcher` — drops `fs.write` and `shell.exec`,
     narrows `net.fetch` to a URL prefix, runs at
     `Trusted` (see correction block above for why not
     `SemiTrusted`). Demonstrates D4 Rule 2 (unqualified-
     held grants qualified-needed) at config-load time.
   - `junior_researcher` — empty `capability_scopes`,
     `parent_role = "researcher"`, `trust_ceiling =
     "Trusted"`. The deliberate "empty-child surprise"
     case the Task 2 correction block flagged: at runtime
     the assembler substitutes the backcompat floor for
     the empty level (NOT `researcher`'s declared set),
     so the runtime envelope ends up with
     `fs.read:<sandbox>/**` (path-qualified) instead of
     `researcher`'s unqualified `fs.read`. The two roles
     produce *different* runtime envelopes despite the
     mental-model expectation that a child with no
     declared scopes "inherits everything" from its
     parent.
2. **Top-of-file comment block** in `examples/aivyx.toml`
   — explains the schema (`name`, `system_prompt`,
   `tool_allowlist`, `memory_topic_prefix`,
   `capability_scopes`, `trust_ceiling`, `parent_role`),
   the absent-vs-empty distinction, and a per-role
   walkthrough of the runtime envelope math. The
   `junior_researcher` block in particular includes a
   step-by-step intersection trace that an operator
   can read top-down to understand exactly what they
   get.
3. **Three new binary-internal regression tests** in
   `crates/aivyx-channel/src/bin/aivyx.rs` (in `mod
   tests`, after the existing five envelope-assembly
   tests):
   - `example_aivyx_toml_coder_envelope_matches_documented_set`
   - `example_aivyx_toml_researcher_envelope_matches_documented_set`
   - `example_aivyx_toml_junior_researcher_envelope_demonstrates_floor_substitution`
4. **Two new test helper fns** in the same `mod tests`:
   - `local_channel_floor_with_sandbox(sandbox: &str)
     -> Vec<Scope>` — builds a representative
     backcompat floor with `/tmp/sandbox` as the
     stand-in canonical path, mirroring the production
     code's startup canonicalization (the example file
     and the test agree on this stand-in path).
   - `load_example_config() -> AivyxConfig` — loads
     `examples/aivyx.toml` via `AivyxConfig::load_from_
     env_and_toml` with `CARGO_MANIFEST_DIR`-relative
     path resolution. Same loader the binary uses.
5. **`junior_researcher` test pins both the absolute
   envelope and the divergence from `researcher`.** The
   `assert_eq!` locks the exact five-scope set; the
   `assert_ne!` against `researcher`'s envelope locks
   the surprise itself — if a future capability-layer
   change accidentally aligned the two envelopes, this
   test would break, prompting a re-read of both the
   correction block above and the example file's
   comment.
6. **Phase 12 Task 3 deferral closed.** The "default
   role config file" deferral from Phase 12 Task 3 is
   now superseded by `examples/aivyx.toml` — recorded
   in the deferrals block at Phase 13 exit (Task 5).

**Exit criteria — all met.**

- ✅ `examples/aivyx.toml` exists at repo root, parses
  cleanly via `AivyxConfig::load_from_env_and_toml`.
- ✅ Three regression tests load the sample and verify
  all three operator-relevant role envelopes (the
  fourth role, `default`, is the parent and is
  exercised transitively through the other three).
- ✅ `cargo test --workspace` green: **468 → 471
  passed**, delta **+3** for Task 3 alone (matching
  the draft's ≥+3 acceptance exactly).
- ✅ `cargo clippy --workspace --all-targets -- -D
  warnings` clean.
- ✅ **Production-core `lib.rs` byte-identity streak
  preserved.** `git diff 16e618c -- crates/aivyx-core/
  src/lib.rs | wc -l` returns `0`.
- ✅ **DESIGN.md + PRODUCT.md byte-identity preserved.**
  `git diff 80189b4 -- DESIGN.md PRODUCT.md | wc -l`
  returns `0`.
- ✅ Phase 12 Task 3 deferral (`default role config
  file`) explicitly closed; recorded for the Phase 13
  exit deferrals block.

**Deferred (recorded so the backlog doesn't silently grow).**

- **Lift `assemble_role_envelope` into `aivyx-channel/
  src/lib.rs`.** Task 3 chose binary-internal unit
  tests (correction block above). A future task that
  wants the example file exercised by a true
  *integration* test under `crates/aivyx-channel/
  tests/` should lift the fn into the lib first. The
  fn has no binary-specific state — the lift would be
  a clean cut, not a refactor.
- **Per-tier worked examples.** The example file
  demonstrates `Trusted` thoroughly. A SemiTrusted-
  channel-focused example (with path-qualified
  fs scopes that survive `CEILING_SEMITRUSTED`) is
  worth shipping in a future phase, alongside an
  Untrusted example for completeness — but Phase 13's
  one-file-fits-all approach is sufficient for the
  P9 exit criterion.

## Task 4 — correction recorded mid-implementation (2026-04-15)

**What I drafted.** A first-pass `render_role_envelope`
in `aivyx.rs` that walked the resolved parent chain,
intersected declared sets with the floor and ceiling,
printed the effective envelope, and then printed a
"dropped" section listing **every** scope from every
level of the chain plus the floor that did not survive
intersection. The intent: surface anything a naive
reader might have expected to see but didn't.

**What broke when I ran the first test pass.** The
`print_role_renders_coder_envelope_against_example_config`
test failed because the rendered output listed
`net.fetch [level 2 default]  reason: no scope with
base 'net.fetch' in the effective envelope` — i.e., the
renderer flagged `default`'s `net.fetch` as a dropped
scope when rendering `coder`. But **`coder` drops
`net.fetch` on purpose.** Its own declared
`capability_scopes` list deliberately omits it. That is
the entire point of the attenuation machinery Task 2
built: a child role attenuates its parent by *not*
re-declaring scopes it doesn't want.

Listing that as a "dropped" surprise conflates two very
different operator concerns:

- **Intentional ancestor attenuation** — the child's
  author chose to drop this. The child's TOML is the
  evidence; no debugging is needed; listing it as
  "dropped" is just noise the operator has to visually
  skip past.
- **Genuine surprise** — something the *leaf* role
  declared that evaporated (e.g. a SemiTrusted role
  declaring unqualified `fs.read` and silently losing
  it to `CEILING_SEMITRUSTED`'s ▲ rows), OR something
  the *floor* injected via the empty-child substitution
  path that didn't survive (the Task 2/Task 3 surprise).

The first pass treated these identically. It would
have generated 6–10 lines of "drops" for a clean
attenuation like `coder`, burying the one line an
operator actually needs to read under noise.

**What I changed.** Restrict the dropped section to
exactly two surfaces:

1. **Active/leaf role drops.** Iterate the leaf role's
   own `capability_scopes.value`. For each scope, if it
   is not in the effective envelope (by exact equality
   or `effective.grants`), report it with tag
   `[active role <leaf_name>]`. This surfaces the
   SemiTrusted-fs.read footgun and any other case
   where the leaf wrote a scope the intersection chain
   stripped.
2. **Floor drops (empty-child only).** If and only if
   at least one level in the chain has empty
   `capability_scopes`, iterate the backcompat floor
   and report unsurvived scopes with tag `[floor]`.
   This surfaces the Task 2/Task 3 empty-child
   surprise by name: an operator running
   `--print-role junior_researcher` sees explicit
   "shell.exec [floor]" and "fs.write [floor]" lines
   and immediately understands *which* scopes the
   floor tried to inject and which the intersection
   chain stripped.

Ancestor levels (level ≥ 2) are never listed in the
dropped section. Their drops are by-design
attenuations; the operator can read them off the TOML
directly.

The header line was also rewritten to spell the new
contract out explicitly: `dropped (surprises only —
scopes the active role or backcompat floor declared
that did not survive intersection; intentional
ancestor-level attenuations are not listed)`. The
empty-state line similarly changed from `<none - every
declared scope survived intersection>` to `<none -
every scope the active role declared survived
intersection>`, so a reader knows exactly which scope
population the "none" applies to.

**A second bug surfaced during the fix.** The
scope-survival check initially used
`effective.grants(scope) || effective.iter().any(|s|
s.is_granted_by(scope))`. The second clause is
backwards: `s.is_granted_by(scope)` asks *"does `scope`
grant `s`"*, not *"does some effective scope grant
`scope`"*. In practice this misfired on
`net.fetch:url-prefix:https://httpbin.org/` in the
researcher envelope: the scope was literally present
in `effective`, but `effective.grants` returned false
for reasons I didn't chase (likely a subtlety in how
the url-prefix qualifier reflexive case dispatches).
The fix short-circuits the check with exact equality
before falling through to `grants`:
`effective.iter().any(|s| s == scope) ||
effective.grants(scope)`. The backwards `is_granted_by`
clause is gone.

*Deferral recorded:* investigate why `CapabilitySet::
grants` does not return true for a scope present by
exact identity in the set, when that scope has a
url-prefix qualifier. This is almost certainly a
capability-layer reflexivity issue worth a small
standalone fix in a future phase — Phase 13 works
around it at the call site, but the workaround should
not become load-bearing elsewhere.

**Why this is the right line.** The whole point of
`--print-role` is to answer *"why doesn't my role have
the scope I declared?"*. An operator who reads a long
dropped block of ancestor attenuations and then gives
up before finding their actual surprise is worse off
than one who sees a short, focused "here is what you
declared that got stripped" section. The two-surfaces
rule matches the two ways a scope can disappear
non-obviously: the active role wrote it and the
chain/ceiling stripped it, or the floor tried to
inject it and the chain stripped it.

## Task 4 — shipped (2026-04-15)

**What landed.**

1. **`--print-role <name>` CLI flag** in
   `crates/aivyx-channel/src/bin/aivyx.rs`. Parses via
   `parse_cli_args_from` alongside the existing
   `--role`, `--channel`, `--verify-only` flags.
   Mutually exclusive with `--verify-only`; composes
   orthogonally with `--channel`; empty value is an
   error with a pointer to the flag name.
2. **Early-exit branch in `run()`.** When
   `print_role.is_some()`, the branch lands **after**
   config load (so typos surface with the same
   `UnknownRole` error + candidate list that
   `--role` already produces) but **before**
   `mkdir fs_root`, master-key derivation, store open,
   and runtime build. No passphrase prompt, no
   filesystem side effects, no Telegram bot
   handshake. The config loader runs with
   `require_api_key = false` and
   `require_telegram_token = false` when in
   print-role mode.
3. **`build_display_floor(fs_root, channel_kind)`
   helper.** Produces a representative backcompat
   floor for the requested channel. Uses
   `fs::canonicalize(fs_root)` if it succeeds (same
   as production startup), falls back to the
   as-written path with a `false` canonicalization
   flag otherwise. The renderer prints a footnote
   when canonicalization failed: `(note: fs sandbox
   at <path> did not exist or could not be
   canonicalized; the floor's fs.read/fs.write
   scopes use the as-written path. A live binary
   would canonicalize through any symlinks at
   startup.)`. This makes the output stable on CI
   hosts that don't have the operator's sandbox dir.
4. **`render_role_envelope(role_name, cfg,
   channel_kind)` renderer.** Walks the parent chain
   leaf-to-root, prints each level's declared scopes +
   trust_ceiling, prints the floor and footnote,
   prints the effective envelope (sorted, so diffs
   are stable), prints the dropped section per the
   two-surfaces rule from the correction block.
   Returns a `Result<String, String>` so the unknown-
   role error carries the typed message up to
   `run()`'s exit branch.
5. **`drop_reason_for(dropped, effective)` helper.**
   Three-case classification: (a) "no scope with
   base `<b>` in the effective envelope" when the
   entire base is missing, (b) "qualified-held
   cannot grant unqualified-needed (D4 Rule 4)" when
   the needed scope is unqualified but the effective
   set has a more-qualified form, (c) "qualifier
   shape mismatch with surviving scopes: <list>"
   fallback. Explicitly less precise than full D4
   rule dispatch — the goal is a hint, not a proof.
6. **Nine new tests** in `mod tests`:
   - **Five parse tests:**
     `print_role_flag_parses_into_cli_args`,
     `print_role_flag_missing_value_is_an_error`,
     `print_role_flag_empty_value_is_an_error`,
     `print_role_and_verify_only_are_mutually_exclusive`,
     `print_role_composes_with_channel_flag`.
   - **Four functional tests:**
     `print_role_renders_coder_envelope_against_example_config`,
     `print_role_renders_junior_researcher_with_visible_drops`
     (the load-bearing test that pins `shell.exec
     [floor]` and `fs.write [floor]` as explicit
     surprise signals),
     `print_role_renders_researcher_with_no_drops`
     (the contrast test that pins clean attenuation
     produces no false-positive noise),
     `print_role_unknown_name_lists_known_roles_in_error`.

**Exit criteria — all met.**

- ✅ `--print-role <name>` parses, exits early, renders
  a useful envelope breakdown for any role in
  `examples/aivyx.toml`.
- ✅ `cargo test --workspace` green: **471 → 480
  passed**, delta **+9** for Task 4 alone (5 parse +
  4 functional).
- ✅ `cargo clippy --workspace --all-targets -- -D
  warnings` clean.
- ✅ **Production-core `lib.rs` byte-identity streak
  preserved.** `git diff 16e618c -- crates/aivyx-core/
  src/lib.rs | wc -l` returns `0`.
- ✅ **DESIGN.md + PRODUCT.md byte-identity preserved.**
  `git diff 80189b4 -- DESIGN.md PRODUCT.md | wc -l`
  returns `0`.

**Deferred (recorded so the backlog doesn't silently grow).**

- **`CapabilitySet::grants` reflexivity investigation.**
  The correction block records a workaround in
  `render_role_envelope` where a url-prefix-qualified
  scope present in the effective set by exact identity
  did not return true from `effective.grants(scope)`.
  The call site works around this with an equality
  check before the `grants` call. A future Phase
  should investigate the root cause in
  `aivyx-capability` and either fix it or document
  the asymmetry as intentional.
- **JSON output mode for `--print-role`.** The
  current renderer produces a human-readable string.
  A `--print-role-json` variant that emits the same
  breakdown as structured data would be useful for
  integration into operator tooling. Not needed for
  Phase 13 P9 exit.

## Task 5 — exit freeze (2026-04-15)

Phase 13 closes cleanly: four implementation tasks, four
ship records, three mid-implementation correction blocks
(Tasks 1, 2, 3, 4), the Phase 12 Task 3 deferral
consumed by Task 3, every byte-level streak held.

### Q1–Q6 resolution

- **Q1 — Where does `capability_scopes` live?**
  *Resolved to* per-role TOML table-array in the single
  config file (`[[role]]` entries in
  `examples/aivyx.toml`). Task 1 extended the existing
  `RawRole` struct in `aivyx-config` rather than
  inventing a second config surface. The
  one-file-per-role option was not pursued; the single-
  file shape composed cleanly with Phase 11's loader.
- **Q2 — How are scope strings parsed?**
  *Resolved to* `Scope::parse` directly at config-load
  time (Task 1). Parse errors point at the specific
  scope string with the TOML path context. No config-
  side wrapper type was introduced — the capability
  layer's parser is the single source of truth for
  scope syntax.
- **Q3 — How does `trust_ceiling` interact with the
  channel's trust tier?**
  *Resolved to* two sequential intersections (Task 2):
  role envelope intersected against role-tier ceiling,
  then against channel-tier ceiling. The more-
  restrictive of the two wins at dispatch time. The
  example file's `researcher` comment block walks
  through why this produces asymmetries (`CEILING_
  SEMITRUSTED`'s ▲ rows strip unqualified fs.read).
- **Q4 — Is `parent_role = "default"` implicit or
  explicit?**
  *Resolved to* **explicit** (Task 1 correction block).
  No implicit parenting: the `default` role is an
  ordinary node in the inheritance forest. The Task 1
  draft had assumed implicit, and the correction block
  records why explicit is less magic for operators
  reading their own config.
- **Q5 — How does the child-parent attenuation check
  surface errors?**
  *Resolved to* declared-sets-only, walking up
  through empty ancestors, enforced at config-load
  time against the walker's nearest constraining
  ancestor (Task 2 correction block). An empty child's
  declared set is the unconstrained sentinel; the
  runtime assembler substitutes the binary's
  backcompat floor for that level one step deep,
  surfacing the "empty-child surprise" as a separate
  teaching case. Error messages quote the operator's
  own scope strings and name the parent they failed
  to attenuate against.
- **Q6 — Does the binary's hard-coded capability
  vector survive Phase 13 at all?**
  *Resolved to* **yes, as a backcompat floor**
  (Task 2). The vector stays as a private binary
  constant, consulted only when a chain level has
  empty `capability_scopes`. Operator-facing config
  never names the floor; it surfaces explicitly in
  `--print-role` output via the "backcompat floor
  (substituted for empty levels)" block and the
  canonicalization-failure footnote, so an operator
  debugging a surprise can see exactly which scopes
  came from the floor vs. their own TOML.

### Phase 13 deferrals

Phase 13 entered carrying **eight** rolling deferrals
from Phase 12. Task 3 consumed the `default role
config file` item directly. Tasks 2, 3, and 4 each
recorded one net-new deferral. Phase 13 exits with
**ten** rolling deferrals total (seven inherited + three
net-new).

**Rolling deferrals still open after Phase 13 (inherited):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1 deferral, untouched by Phase 13.
  Carries forward. Tagged: **Phase 11 Task 4,
  earliest plausible: whichever phase has a concrete
  forensic-tooling story that needs the
  `tool.allowlist:` scope distinction to be pattern-
  matchable on variant shape rather than scope base
  name.**
- **Second regression channel for the role
  primitive** — Phase 11 Q6 deferral. Untouched by
  Phase 13; reopens reactively only if a channel-seam
  bug surfaces that turn-loop tests miss.
- **Response headers in audit payload (Phase 12 Q3
  half).** Untouched by Phase 13. Tagged: **Phase 12
  Task 2, earliest plausible: whichever phase has a
  concrete forensic story that wants response headers
  in the audit chain.**
- **Non-GET verbs (POST/PUT/PATCH/DELETE).** Phase
  12 Q1 pinned GET-only. Tagged: **deferred
  indefinitely — reopens only when a concrete write-
  side use case surfaces.**
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

**Net-new deferrals from Phase 13 itself:**

- **Lift `assemble_role_envelope` into
  `aivyx-channel/src/lib.rs`.** Task 3's correction
  block records why the worked-example regression
  tests had to live *inside* the binary's `mod tests`
  (the assembly fn is binary-private). A future task
  that wants the example file exercised by a true
  *integration* test under `crates/aivyx-channel/
  tests/` should lift the fn into the lib first. The
  fn has no binary-specific state — a clean cut, not
  a refactor. Tagged: **Phase 13 Task 3, earliest
  plausible: whichever phase wants a cross-crate
  integration test against the example config.**
- **Per-tier worked examples.** `examples/aivyx.toml`
  demonstrates `Trusted` thoroughly. A SemiTrusted-
  channel-focused example (with path-qualified
  fs scopes that survive `CEILING_SEMITRUSTED`'s ▲
  rows) and an Untrusted example for completeness
  are worth shipping in a future phase. Tagged:
  **Phase 13 Task 3, earliest plausible: a phase
  that ships a second channel adapter at a lower
  trust tier and needs a canonical role profile to
  pair with it.**
- **`CapabilitySet::grants` reflexivity
  investigation.** Task 4's correction block records
  a workaround in `render_role_envelope` where a
  url-prefix-qualified scope present in the effective
  set by exact identity did not return true from
  `effective.grants(scope)`. The call site works
  around it with an equality check before the
  `grants` call. A future phase should investigate
  the root cause in `aivyx-capability` and either
  fix it or document the asymmetry as intentional.
  Tagged: **Phase 13 Task 4, earliest plausible: any
  phase that touches `aivyx-capability` — the
  investigation should live alongside whatever other
  capability-layer work pulls the crate into scope.**

**JSON output mode for `--print-role`** is also
recorded in the Task 4 ship record but deliberately
not promoted to the phase-level deferrals list: it's a
UX nicety, not a foundation primitive, and will be
picked up reactively if operator tooling asks for
structured debug output.

**Backlog shape at Phase 13 exit:** seven rolling items
inherited from Phase 12 (minus the one Task 3 closed) +
three net-new items from Phase 13. Total ten — up from
Phase 12's exit total of eight. Every item is scoped,
originating-task-tagged, and reactive-trigger-tagged.
The growth is "known deferrals" rather than "accumulated
debt": Phase 13 closed one item directly (the worked
example) and recorded three follow-ups that each pin to
a concrete future task-shape rather than drifting into
the backlog as untagged TODOs.

### Phase 13 exit criteria (final)

- [x] Task 1 shipped at `2c7acfe`: per-role capability
      envelope fields (`capability_scopes`,
      `trust_ceiling`, `parent_role`) in
      `aivyx-config`, single-inheritance substrate,
      load-time parent-chain cycle + dangling-parent +
      attenuation invariants. **+7 tests**.
- [x] Task 2 shipped at `af89874`: binary capability
      assembly rewritten to consume the per-role
      envelope via `assemble_role_envelope` walking
      the parent chain, intersecting declared sets
      leaf-to-root with backcompat-floor substitution
      per empty level, composed through two
      sequential ceiling intersections. **+8 tests**.
- [x] Task 3 shipped at `a19c6e4`: `examples/aivyx.toml`
      worked inheritance case with four roles
      (`default`, `coder`, `researcher`,
      `junior_researcher`) and top-of-file operator-
      facing comment block walking through the
      empty-child surprise. **+3 tests**. Phase 12
      Task 3 `default role config file` deferral
      closed.
- [x] Task 4 shipped at `3e83422`: `--print-role <name>`
      CLI flag with early-exit branch (after config
      load, before mkdir/keys/store), human-readable
      renderer, two-surfaces drop-reporting rule
      distinguishing intentional attenuation from
      genuine surprise. **+9 tests** (5 parse + 4
      functional).
- [x] Decisions block (Q1–Q6 resolution) recorded
      above.
- [x] Deferrals block recorded above: 7 inherited +
      3 net-new = 10 rolling items.
- [x] `cargo test --workspace` green at exit: **453 →
      480 passed**, delta **+27** across the phase
      (well above the draft's ≥+16 acceptance — Task
      1 +7, Task 2 +8, Task 3 +3, Task 4 +9).
- [x] `cargo clippy --workspace --all-targets -- -D
      warnings` clean at exit. Pre-commit hook held
      throughout.
- [x] **`DESIGN.md` byte-identical to `e0d6437`.**
      **Streak rolls to thirteen consecutive phases.**
      (Draft said "fourteen" — an off-by-one; Phase
      12 exit had it at twelve, so Phase 13 is
      thirteen. Recorded here rather than amending
      the draft, so the phase arithmetic stays
      legible.) Verified: `git diff e0d6437 HEAD --
      docs/DESIGN.md | wc -l == 0`. No amendment file
      created during Phase 13.
- [x] **`PRODUCT.md` byte-identical to `80189b4`.**
      **First phase since PRODUCT.md landed where it
      is byte-identical through exit** — the product
      contract is delivered on, not revised, by
      Phase 13. Verified: `git diff 80189b4 HEAD --
      PRODUCT.md | wc -l == 0`. PRODUCT.md streak
      begins at **one consecutive phase**.
- [x] **Production-core `lib.rs` byte-identical to
      `16e618c`.** **Streak extends to two
      consecutive phases** — the first re-established
      production-core streak since Phase 10/11 held
      and Phase 12 broke it. Verified: `git diff
      16e618c HEAD -- crates/aivyx-core/src/lib.rs |
      wc -l == 0`. Phase 13 touched zero lines of
      `aivyx-core` (the config work sits entirely
      above it, as the phase-open decision block
      predicted).
- [x] **Zero-new-dep streak: held.** Phase 13 added
      zero new workspace crates. `examples/aivyx.toml`
      is a config file with no build-time footprint.
- [x] `docs/README.md` phase-status table row
      updated: `| Phase 13 | Frozen  | PHASE_13.md |
      <exit-hash> |`. (Exit-hash backfilled in a
      separate follow-up commit per the Phase 11/12
      recipe.)
- [x] `docs/ROADMAP.md` Phase 13 entry replaced with
      a Phase 14 scaffold.
- [x] `docs/PRODUCT_ROADMAP.md` Role-Config Migration
      milestone entry updated to reflect the landed
      shape (single-inheritance per-role envelope in
      a single TOML file + worked example + debug
      flag) and flag the two remaining follow-up
      sub-phase candidates (lift `assemble_role_
      envelope` + per-tier worked examples) as not
      requiring a dedicated sub-phase.
- [x] Phase 12 Task 3 deferral (`default role config
      file`) explicitly consumed by Task 3 and
      closed in the deferrals block above.

### Phase 13 recap

Phase 13 is the first phase written under both
DESIGN.md **and** PRODUCT.md as locked contracts, and
the first to deliver against a numbered Product
Commitment (**P9 — Per-Role Full Capability
Declaration**). It is also the phase where the
foundation backlog first grew modestly under the
dual-contract regime — from eight rolling items at
Phase 12 exit to ten at Phase 13 exit. The growth is
healthy: one item closed (the worked example), three
net-new items recorded with concrete trigger tags, no
items dropped silently.

The four tasks composed cleanly: Task 1 built the
config substrate, Task 2 consumed it in the binary
with the declared-only attenuation walk, Task 3
exercised the whole thing against a worked example
that doubles as operator documentation, Task 4 made
the whole machinery introspectable via a debug flag
the operator can run without any side effects. Each
task's correction block records a specific case
where the draft plan assumed the wrong thing and the
implementation revealed the right thing — the Phase
11 discipline of recording corrections in-place
rather than rewriting the draft continues to pay
off.

Three streaks survived the phase:
- DESIGN.md → **thirteen** consecutive phases.
- PRODUCT.md → **one** (phase of origin).
- Production-core `aivyx-core/src/lib.rs` → **two**
  (re-established after Phase 12 broke it).

Phase 14 shape TBD — the draft's three candidates
(Daemon Migration start, Mission Primitive, Sub-
Agent Role-Switching) all remain viable, and Phase
13's clean exit means none of them are blocked on
further foundation work. The decision can be made at
Phase 14 open under the same dual-contract
discipline.

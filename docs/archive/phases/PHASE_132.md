# Phase 132 — `aivyx-auth-cli` Substrate Lift

**First substrate phase since Phase 129** (which lifted
the Google OAuth helpers into `aivyx-google-oauth`).
Operator-picked at the Phase 131 direction question
after the auth_cli posture reached **three concrete
data points** (`aivyx-notion`, `aivyx-obsidian`,
`aivyx-n8n`) all carrying near-identical slim
non-OAuth auth_cli surfaces.

## Why this, why now

- **Three reinforcing data points.** Phase 130 (Notion,
  Obsidian) flagged the lift candidate; Phase 131 (n8n)
  validated it; Phase 132 ships it. The
  per-consumer auth_cli is ~400 LoC of structurally
  identical code that drifts independently in three
  crates today. Lifting it now ahead of Chapter F #8
  (GitHub) means the next integration starts from the
  substrate.

- **Net LoC reduction.** Pre-lift: 1177 LoC across the
  three `auth_cli/` directories. Post-lift target: a
  ~250 LoC `aivyx-auth-cli` substrate plus ~50 LoC of
  consumer adapters per crate (3 × 50 = 150). Net delta:
  ~-770 LoC, two-thirds of which is the shared
  `parse_cli_args` + `ConfigFileError` + `StatusReport`
  / `CheckReport` machinery operators see in every
  Chapter F crate.

- **Template for Chapter F #8.** GitHub will use a
  Personal Access Token (PAT) with the same slim
  auth_cli shape — `aivyx-github auth status` + `auth
  check` against `/user`. Phase 132 makes the GitHub
  crate's auth_cli a ~50 LoC adapter from day one.

- **Auth_cli lift posture, locked.** Phase 130 +
  Phase 131 each predicted this lift in their exit
  docs; the third data point is now in. The
  substrate-lift-at-3-data-points heuristic is
  consistent with the multi-tool-harness lift (Phase
  128 — two data points was the threshold there
  because the duplication was higher-volume).

## Q-block sign-off (2 Recommended)

- **Q1a — Shared types + helpers** (Recommended; operator-picked).

  Lift granularity is "shared types + reusable
  helpers," **not** a full trait that consumers
  implement. The substrate owns the type definitions
  and the structurally identical pieces (CLI argument
  parsing, TOML loading, default config path
  computation, report Display impls); the consumer
  owns its `Config` struct, `help_text()`, service-
  specific validation, and `check()` body.

  Rejected alternative (Q1b): "Full trait lift —
  `ServiceAuth` with a generic `run_auth_main`." Would
  collapse the per-service main.rs further but forces
  uniformity across three meaningfully different
  shapes (Obsidian's check is filesystem-only; Notion
  + n8n are HTTP). Conservative choice: lift the
  duplication, not the structure.

  Rejected alternative (Q1c): "Hybrid — trait for
  status/check, manual for IpcLoop." Same forced
  uniformity problem; more complex implementation.

- **Q2a — New crate `aivyx-auth-cli`** (Recommended;
  operator-picked over Q2b's "add to `aivyx-tool`").

  The auth surface is conceptually separate from the
  IPC multi-tool harness (Phase 128's lift target);
  bundling them would conflate two substrate concerns
  the operator might want to tune independently.
  Workspace already carries 22 crates; one more for a
  ~250 LoC substrate is acceptable.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; the lift is purely below-D1, below-D4
  substrate consolidation. Current hash:
  `62dabbdd…`. Prediction: streak **extends to 23**
  (currently 22 after Phase 131).

- **PRODUCT.md** — **Will hold.** P10/P11/P12 already
  cover third-party tool processes; substrate lifts
  inside those tool processes don't move product
  scope. Current hash: `467ba59a…`. Prediction:
  streak **extends to 23**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** The
  lift lives entirely under the
  Chapter-F-tool-process boundary; core stays
  untouched. Current hash: `4f9b8c81…`. Prediction:
  streak **extends from 5 to 6**.

## Tasks

1. **Open doc + ROADMAP + README** — this doc; the
   roadmap section; the README row at "Frozen" once
   the exit doc lands.

2. **`aivyx-auth-cli` crate** — new workspace member.
   Surface:
   - `BinaryMode { Help, Auth(AuthMode), IpcLoop }` +
     `AuthMode { Status, Check }`.
   - `parse_cli_args(argv, binary_name) -> Result<BinaryMode, String>`.
   - `ConfigFileError` enum: `NotFound { path }`,
     `Io { path, source }`, `Parse { path, reason }`.
     Service-specific "Empty*" / "NotAbsolute" / etc.
     stay on the consumer side as a separate enum
     that composes with this one.
   - `default_config_path(service_subdir: &str) -> Option<PathBuf>`
     — computes `$HOME/.aivyx/tool-processes/<subdir>/config.toml`.
   - `load_toml<T: DeserializeOwned>(path) -> Result<T, ConfigFileError>`
     — IO + parse, no service-specific validation.
   - `StatusReport { config_path, binary_name, ok, detail }`
     + Display.
   - `CheckReport { binary_name, ok, message }` + Display.

3. **Migrate `aivyx-notion`** — replace internals of
   `aivyx-notion/src/auth_cli/{cli,config_file,status}.rs`
   with thin wrappers that pull from `aivyx-auth-cli`.
   Public API surfaces (`parse_cli_args_from`,
   `load_config`, `run_auth_status`, `run_auth_check`)
   preserved so callers don't change.

4. **Migrate `aivyx-obsidian`** — same pattern. Obsidian
   has `check.rs` instead of `status.rs` (no remote
   API to check) and its config carries a vault path,
   not a token. The substrate's `CheckReport` shape
   still fits.

5. **Migrate `aivyx-n8n`** — same pattern. Most
   straightforward of the three because Phase 131
   already wrote it with this lift in mind.

6. **Exit doc + INSTALL substrate note + prediction-vs-reality.**
   The "Building a new Chapter F integration" section
   in INSTALL.md grows a paragraph pointing at
   `aivyx-auth-cli` as the substrate to start from.

## Exit criteria

- [ ] `docs/PHASE_132.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `aivyx-auth-cli` crate with comprehensive tests —
  Task 2.
- [ ] Three consumer crates migrated; their existing
  tests pass unchanged — Tasks 3-5.
- [ ] Net workspace LoC delta is negative.
- [ ] DESIGN.md / PRODUCT.md / lib.rs all HOLD as
  predicted.
- [ ] Zero new workspace dependencies (the substrate
  uses `serde` + `toml` + `thiserror`, all already
  present).
- [ ] Zero clippy warnings.
- [ ] Test count delta is approximately neutral
  (substrate gains ~25 tests; consumers lose ~20 each
  because the same logic is now tested once at the
  substrate level).

## Honest scope risks at sign-off

- **API churn in three consumers.** Three crates
  change their internal `auth_cli/` files in one
  phase. The public exports stay stable so the binary
  CLIs work identically, but the internal module
  shapes shift. Mitigation: lift first (Task 2), then
  migrate one consumer at a time (Tasks 3-5), each
  task self-contained behind a green test run.

- **Test consolidation may surface differences.**
  Each consumer's `auth_cli/` has its own tests today
  (~20-30 per crate). Lifting the substrate moves
  ~15 of those to `aivyx-auth-cli` as substrate
  tests. The remaining ~10-15 per consumer cover
  service-specific behaviour (Notion empty-token,
  Obsidian path-must-be-absolute, n8n empty-base-url).
  If the lift exposes inconsistent tests across the
  three (e.g. Obsidian validated one thing the others
  didn't), the consolidation step picks a single
  consistent behaviour and updates the outliers.

- **`ConfigFileError` boundary.** The substrate's
  error enum carries the path-agnostic variants
  (NotFound / Io / Parse). Service-specific validation
  errors (EmptyToken, NotAbsolute) stay on the
  consumer side as a separate enum. Tests need to
  cover both error layers without confusing them.

- **First-substrate-phase-since-129 ergonomics.**
  Phase 129's `aivyx-google-oauth` lift was the last
  substrate consolidation; Phase 132 reuses the same
  shape (new crate, three consumers, fan-out
  migration). If anything about Phase 129's
  cross-crate workflow needs revisiting, this phase
  is the natural place.

- **Twenty-first consecutive deferral of the Channel
  Activation Milestone** if Phase 132 ships without
  taking it. The deferral count continues to grow;
  honest tracking continues.

## Direction after Phase 132

After Phase 132, Phase 133 candidates:

1. **Channel Activation Milestone** — twenty-first
   deferral if skipped. The substrate-quality work
   keeps adding up; the milestone work is now the
   stand-out gap.
2. **Chapter F #8 — GitHub** — first integration to
   pilot the new `aivyx-auth-cli` substrate. Would
   verify the lift's design under a fresh consumer
   that wasn't part of the migration.
3. **Release prep (v0.1.0 + installer)** — substrate
   foundation now reaches a "release-ready" level.
4. **Audit pass #2** — pick another subsystem
   (channels? mission state machine? skill auto-
   proposer?) and run the same audit-and-fix workflow
   the Agent Loop pass just demonstrated.

## Prediction vs reality

**Three-of-three streak predictions correct.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  22 → **23**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). P10/P11/P12 framing covers Chapter F
  substrate work without revision. Streak: 22 → **23**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). The lift lives entirely
  under the tool-process boundary. Streak: 5 → **6**.

**Test count delta: +3 — landed within the
"approximately neutral" prediction.** Workspace lib
tests 2963 → 2966. Substrate added 19; the three
consumers shed 16 total (notion 0, obsidian -3, n8n
-5; cli-argument-parsing tests now live in the
substrate). The net-positive result reflects that the
substrate adds new coverage (binary-name threading,
the type-mismatch case for `load_toml`) that none of
the three consumers carried individually before the
lift.

**Zero new workspace dependencies** as predicted. The
substrate uses serde + toml + thiserror, all already
in the workspace.

**Zero clippy warnings** workspace-wide.

### LoC delta missed the prediction

The open doc projected a net **-770 LoC** delta. The
actual delta is **+325 LoC** (1177 → 1502 across the
four auth_cli locations). Honest framing of the miss:

- The substrate's own tests are ~270 LoC — coverage
  the open doc undercounted in its sketch.
- Each consumer's wrapper code (ConfigFileError
  composition enum, `From` impls, default_config_path
  shim, service-specific tests) adds ~10-15% over
  what a naive "delete and re-import" would.
- The substrate gained new coverage that wasn't in
  any pre-lift consumer: binary-name threading
  tests, `load_toml` type-mismatch tests, etc.

**The lift's real wins are structural, not size-based.**

- **Single source of truth for argument parsing.** A
  future bug fix in the `auth <subcommand>` parser
  lands in one place. Pre-lift, the same fix would
  have needed three near-identical PRs.
- **Consistent error shapes across consumers.** Every
  consumer's `NotFound` / `Io` / `Parse` errors
  produce identical wording. Operators reading
  diagnostics across multiple Chapter F crates see
  one error vocabulary instead of three.
- **`BinaryMode` shape alignment.** Obsidian's pre-
  lift CLI had a slightly different surface
  (`Help | Check | IpcLoop` with `check` shorthand);
  the migration aligned it with notion + n8n so
  operators learning one tool's CLI know all three.
- **Future Chapter F integrations start from the
  substrate.** When Chapter F #8 (GitHub) lands, its
  auth_cli/ is a ~50 LoC adapter — vs the ~400 LoC
  per-service implementations Phase 130 + 131
  shipped.

### Auth_cli lift posture, locked

Three data points (notion + obsidian + n8n) all
migrated cleanly with the same wrapper shape. The
substrate-lift-at-3-data-points heuristic is now an
established pattern alongside the multi-tool-harness
lift (Phase 128 at 2 data points). The threshold for
the next lift candidate (whatever it is) gains
empirical weight.

### Twenty-first consecutive deferral of the Channel
Activation Milestone

Honest tracking continues. Phase 133 direction-after
flags the milestone as the leading candidate; the
deferral count's signal-strength is now in its 21st
consecutive phase.

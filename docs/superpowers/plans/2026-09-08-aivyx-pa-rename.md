# Aivyx → Aivyx PA Rename Implementation Plan (Sub-project A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the flagship product from "Aivyx" to "Aivyx PA" — the
technical identity (binary, config/data paths, embedded string constants)
and the product-name prose across every live doc in this repo — while
"Aivyx" stays the org/ecosystem name and the repo's own git history
(including `docs/archive/phases/` and past `CHANGELOG.md` entries) stays
untouched as accurate history.

**Architecture:** All code and docs changes happen inside this repo at its
current path/name (`~/Projects/Rust/aivyx/`) — the repo/directory's own
rename to `aivyx-pa` happens as a **separate, manual, post-merge step**,
never mid-implementation, so no task works against a moving path. Design
spec: `docs/superpowers/specs/2026-09-08-aivyx-pa-rename-design.md` —
read it in full before starting; this plan assumes it, plus one correction
made during this plan's own grounding pass (see Global Constraints).

**Tech Stack:** Rust (one `[[bin]]` rename + a real audited set of
embedded string constants, ~110 locations plus a ~35-site assistant-
persona-name cluster — see Task 1), Markdown docs (9 Tier-1 files plus 96
Tier-2 files — corrected down from an original ~192 estimate once
`docs/superpowers/`'s 99 historical planning files were excluded; see
Global Constraints).

## Global Constraints

- Naming: **"Aivyx PA"** (space, capital PA) in all prose/documentation;
  **`aivyx-pa`** (lowercase, kebab-case) for the binary name and any
  filesystem path segment.
- The per-occurrence judgment rule for every docs task: refers to the
  specific product/software/daemon/binary → becomes "Aivyx PA." Refers to
  the ecosystem, the org, the `Aivyx-Agent` GitHub org, or brand language
  in the abstract → stays "Aivyx." When genuinely unsure, prefer leaving
  it as "Aivyx" and flag the specific sentence in your task report rather
  than guessing wrong in the more disruptive direction.
- **Never rename**: any of the 34 internal Cargo crate names
  (`aivyx-core`, `aivyx-capability`, `aivyx-storage`, etc. — these stay
  exactly as they are, in both code and docs prose that mentions them by
  name, e.g. "the `aivyx-core` crate" stays exactly that). Never touch
  `docs/archive/phases/*.md` (212 files, frozen historical journals).
  Never touch any *past* `CHANGELOG.md` entry (only the new entry
  documenting this rename is added — see the correction below). Never
  touch the `Aivyx-Agent` GitHub org name, or any reference to sibling
  repos (`aivyx-coder`, `aivyx-broker`, `aivyx-yubi`, etc.) by their own
  names — those are unaffected by this project.
- **Correction made during this plan's grounding, not in the original
  spec**: `CHANGELOG.md` (1,849 lines, a running historical record of past
  releases) gets the *same* frozen-history treatment as
  `docs/archive/phases/` — past entries accurately describe what the
  product was called at the time they shipped, and rewriting them would
  be revising history. Only a **new entry documenting this rename itself**
  is added; no existing entry is edited.
- **Second correction, surfaced by Task 1's own real grounding, not
  anticipated by the original spec**: `docs/superpowers/` (plans, specs,
  and artifacts — this repo's own design/implementation-planning
  process, including specs for other already-shipped work written under
  the product's old name, and even this rename's own spec/plan) gets the
  *same* frozen-history treatment as `docs/archive/phases/` and
  `CHANGELOG.md` — never rewritten. These are historical planning
  records, not living product documentation; rewriting them would
  misrepresent when the rename actually happened. This shrank the
  originally-estimated ~192-file Tier-2 docs set down to the **96 files
  that are genuinely live product documentation** — see Task 1's own
  audit output for the exact accounting.
- **Third resolution, also surfaced by Task 1's grounding**: the
  assistant's own default spoken/display persona name
  (`DEFAULT_ASSISTANT_NAME` and ~35 downstream sites — notification
  titles, the desktop app's window title/tray tooltip, the web Studio's
  wordmark, generated TOML, etc.) is a distinct identity axis from the
  product's technical name, and the design spec never addressed whether
  it renames too. Resolved directly by the operator: **yes** — it becomes
  `"Aivyx PA"` by default, still fully overridable per-operator via
  `[profile] assistant_name` exactly as today. Two path-segment
  exceptions in the same audit cluster (`aivyx-sandbox` →
  `aivyx-pa-sandbox`) follow the technical kebab-case identifier instead,
  since they're filesystem paths, not prose — see
  `docs/superpowers/plans/rename-audit/embedded-constants.md`'s own
  per-row notes for the exact reasoning on each.
- The config filename itself (`aivyx.toml`, referenced throughout docs as
  `~/.config/aivyx/aivyx.toml` or a CWD-relative `./aivyx.toml`) is also
  in scope — it becomes `aivyx-pa.toml`, consistent with the directory
  rename, since it's the product's own default filename. Ground the exact
  literal string constant(s) in Task 1; every docs task that mentions
  `aivyx.toml` updates it to `aivyx-pa.toml` in the same pass as the
  surrounding "Aivyx" → "Aivyx PA" prose changes.
- No test in this plan should need editing for the rename itself beyond
  what Task 1's audit surfaces — internal crate names don't move, so the
  existing dependency graph and test suite are otherwise untouched.
- No auto-migration code, no compat shim binary — this is a documented
  clean break (see Task 3).
- Repo scope: every task in this plan lives in this repo (`aivyx`, soon
  `aivyx-pa`), at its current path. The GitHub repo rename and local
  directory rename happen once, manually, after every task below is
  merged — not as one of the numbered tasks, and not mid-plan.

---

### Task 1: Ground the real current state — audit embedded constants, split INSTALL.md, batch the Tier-2 docs

This is pure grounding/prep — no runtime behavior change, no docs prose
edited yet. Every later task depends on this one's outputs being accurate.

**Files:**
- Create: `docs/superpowers/plans/rename-audit/embedded-constants.md`
  (the audit report)
- Create: `docs/superpowers/plans/rename-audit/install-split-point.md`
  (confirms/adjusts the split line for Tasks 8/9)
- Create: `docs/superpowers/plans/rename-audit/tier2-batch-1.md` through
  `tier2-batch-4.md` (the Tier-2 file-list batches for Tasks 10–13)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: the four artifact types above, which Tasks 2 and 8–13
  directly read as their own requirements.

- [ ] **Step 1: Audit embedded string constants carrying the product
  identity**

Search the whole `crates/` tree (not `docs/`) for literal `"aivyx"` used
as: an XDG config/data/state directory-name segment, the OS keyring
service name (already confirmed real: `crates/aivyx-channel/src/
keyring_store.rs`'s `const SERVICE: &str = "aivyx";`), the daemon socket
filename/path, any systemd/launchd service unit name or template, any
`AIVYX_*`-prefixed env var whose prefix is the literal identity (not
crate-internal), the default config filename (`aivyx.toml`), and any log
file path. Ground this precisely — grep broadly
(`grep -rn '"aivyx"' crates/*/src/**/*.rs`, then narrow), then read each
hit's real context to classify it as: (a) a genuine product-identity
string in scope for this rename, or (b) something that merely contains
the substring "aivyx" as part of an internal crate name, an unrelated
sibling-repo reference, or a test fixture that should NOT change. Write
the in-scope hits to `embedded-constants.md` as a table:
`file:line | current literal | what it's for | new literal`.

- [ ] **Step 2: Find the real config-path-construction code**

The `~/.config/aivyx/`, `~/.local/share/aivyx/`, `~/.local/state/aivyx/`
paths are almost certainly built via a `directories`/`ProjectDirs`-style
crate call with `"aivyx"` as the qualifier/organization/application
argument, likely in `aivyx-config`. Find the exact real call site(s) and
add them to `embedded-constants.md` with the same table shape.

- [ ] **Step 3: Confirm/adjust the `docs/INSTALL.md` split point**

This plan's own grounding found `## Vertical packs — install a signed
pack (Chapter Freight)` at line 3212 as a clean heading boundary near the
file's true midpoint (6,678 total lines). Confirm this line number is
still accurate (re-run `grep -n "^## " docs/INSTALL.md` — content may
have drifted since this plan was written) and write the confirmed exact
line number and heading text to `install-split-point.md`. If the line has
moved, pick the nearest `##`-level heading to the new midpoint instead and
record that one.

- [ ] **Step 4: Generate and batch the Tier-2 docs file list**

Tier 1 (handled explicitly by name in Tasks 3–9) is: `README.md`,
`VISION.md`, `PRODUCT.md`, `DESIGN.md`, `TRADEMARK.md`, `COMMERCIAL.md`,
`docs/THREAT_MODEL.md`, `docs/INSTALL.md`, `docs/ONBOARDING.md`.
(`CHANGELOG.md` is handled separately per the Global Constraints
correction — grouped into Task 3 as a small addition, not a full rewrite,
so exclude it from both the Tier-1 list above and the Tier-2 batch.)

Run:
```bash
grep -rl "Aivyx" --include="*.md" . docs/ 2>/dev/null | grep -v '^docs/archive/' | sort -u
```
(adjust the exact invocation as needed to correctly cover both repo-root
`*.md` and everything under `docs/` while excluding `docs/archive/` —
verify the real command actually produces a sane list before trusting
it, e.g. sanity-check the total count is roughly 190–195 files matching
this plan's own earlier grounding). Remove the 9 Tier-1 filenames and
`CHANGELOG.md` from the list. Split the remainder into 4 roughly-equal
batches (alphabetically or by directory — your choice, whichever produces
more thematically-coherent batches for a reviewer to read through). Write
each batch's file list, one path per line, to
`tier2-batch-1.md` through `tier2-batch-4.md`.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/plans/rename-audit/
git commit -m "docs: audit embedded rename constants, split INSTALL.md, batch Tier-2 docs"
```

---

### Task 2: Rename the binary + fix audited embedded constants

The one task in this plan that changes real runtime behavior.

**Files:**
- Modify: `crates/aivyx-cli/Cargo.toml` (the `[[bin]] name` field)
- Modify: every file/line listed in Task 1's `embedded-constants.md`
- Modify: any existing test that asserts on one of the literal strings
  being changed (Task 1's audit should have flagged these; if it missed
  one, find it via the compiler/test failures this task's own build
  surfaces)

**Interfaces:**
- Consumes: `docs/superpowers/plans/rename-audit/embedded-constants.md`
  (Task 1).
- Produces: a workspace that builds a binary literally named `aivyx-pa`,
  reads/writes `~/.config/aivyx-pa/`, and uses `aivyx-pa.toml` as its
  default config filename.

- [ ] **Step 1: Read `embedded-constants.md` in full**

This is your literal task list — every row is one change to make.

- [ ] **Step 2: Change the binary name**

In `crates/aivyx-cli/Cargo.toml`, find the `[[bin]]` block and change
`name = "aivyx"` to `name = "aivyx-pa"`. Leave `path = "src/bin/aivyx.rs"`
unchanged (the source *file* name is an internal detail, not part of this
rename's user-facing scope) unless the audit found a reason it must move.

- [ ] **Step 3: Apply every other row from the audit**

For each `file:line | current literal | new literal` row, make the exact
change. Prefer touching only the literal string, not surrounding logic.

- [ ] **Step 4: Run the full workspace build and test suite**

```bash
cargo build --workspace
cargo test --workspace
```

Confirm the produced binary is genuinely named `aivyx-pa`
(`ls target/debug/aivyx-pa` or equivalent) and that no crate name in the
build output changed (spot-check `cargo build --workspace 2>&1 | grep
Compiling` still lists `aivyx-core`, `aivyx-capability`, etc. unchanged).

- [ ] **Step 5: Manual runtime confirmation**

Run the freshly-built binary's init/config-write path (however this
repo's own dev-run scripts or `aivyx-pa init` work — check
`scripts/dev-run.sh` for the established pattern) against a scratch
`HOME` and confirm it writes to `~/.config/aivyx-pa/` (and the
`aivyx-pa.toml` filename), not the old paths. This is the one behavior
change a real user would observe, so confirm it directly rather than
only via code review.

- [ ] **Step 6: `cargo clippy --workspace --all-targets -- -D warnings`, then commit**

```bash
git add crates/
git commit -m "feat: rename binary to aivyx-pa, update embedded identity constants"
```

---

### Task 3: `README.md` + `TRADEMARK.md` + `COMMERCIAL.md` + new `CHANGELOG.md` entry

The shortest, highest-stakes Tier-1 group — the legal/marketing-adjacent
front door.

**Files:**
- Modify: `README.md` (395 lines)
- Modify: `TRADEMARK.md` (30 lines)
- Modify: `COMMERCIAL.md` (88 lines)
- Modify: `CHANGELOG.md` (add one new entry only — do not touch any
  existing entry)

**Interfaces:**
- Consumes: the per-occurrence judgment rule (Global Constraints).
- Produces: nothing consumed by a later task — these four files are not
  referenced by any other task's own requirements.

- [ ] **Step 1: `README.md`**

Read in full. Apply the per-occurrence rule throughout — this file is
almost entirely product-referring ("Aivyx is a Rust-built agent
framework..." → "Aivyx PA is a Rust-built agent framework..."), with a
few genuine org/GitHub-org references to leave alone (e.g. any mention of
the `Aivyx-Agent` GitHub org itself, or "Aivyx" used as brand/trademark
language distinct from the product). Update every `aivyx.toml` mention to
`aivyx-pa.toml` and every literal `aivyx` CLI-invocation example
(`./target/release/aivyx`, `aivyx init`, etc.) to `aivyx-pa`/
`aivyx-pa init`.

- [ ] **Step 2: `TRADEMARK.md`**

Read in full. This file needs more than word-swapping — per the design
spec, it should now protect **both** "Aivyx" (the org/brand) and
"Aivyx PA" (the product) as names nobody may use for a confusingly
similar fork. Add explicit language to that effect (e.g. extend "Call
your fork 'Aivyx' or any confusingly similar name" to also name
"Aivyx PA"), rather than only substituting the existing product
references.

- [ ] **Step 3: `COMMERCIAL.md`**

Read in full. Apply the per-occurrence rule — this file describes the
commercial-licensing gate for using *the product*, so most "Aivyx"
occurrences here are product-referring and become "Aivyx PA." The
license itself (BUSL-1.1) and its terms are unaffected by this rename;
only the product name within the prose changes.

- [ ] **Step 4: `CHANGELOG.md` — add one new entry, touch nothing else**

Read the file's existing header/format conventions (don't read the whole
1,849 lines — just enough to match the established entry format). Add a
new entry at the top, above the most recent existing entry, documenting
this rename plainly: the product is now called Aivyx PA, the binary is
`aivyx-pa` not `aivyx`, config/data paths moved from `~/.config/aivyx/`
to `~/.config/aivyx-pa/`, this is a breaking rename with no
auto-migration, and anyone with an existing install who wants to keep
their data should manually move it. Do not edit, reformat, or rename
anything inside any existing entry.

- [ ] **Step 5: Commit**

```bash
git add README.md TRADEMARK.md COMMERCIAL.md CHANGELOG.md
git commit -m "docs: rename Aivyx to Aivyx PA in README, trademark, commercial, changelog"
```

---

### Task 4: `VISION.md` + `docs/ONBOARDING.md`

**Files:**
- Modify: `VISION.md` (148 lines)
- Modify: `docs/ONBOARDING.md` (169 lines)

**Interfaces:** consumes the per-occurrence rule; produces nothing
consumed elsewhere.

- [ ] **Step 1: `VISION.md`**

Read in full. This is the "north star" document — "Aivyx is a
self-learning agentic *personal* assistant..." and similar are all
product-referring. Watch for the one place it discusses the long-term
"network of agents" vision (Nonagon → Nexus) — that section describes
future *products* built on the platform (Nexus, Factory), which are
separate names entirely and are never renamed to anything Aivyx-PA-shaped;
don't touch those names.

- [ ] **Step 2: `docs/ONBOARDING.md`**

Read in full. Operator-facing setup-flow doc — same rule, likely almost
entirely product-referring.

- [ ] **Step 3: Commit**

```bash
git add VISION.md docs/ONBOARDING.md
git commit -m "docs: rename Aivyx to Aivyx PA in VISION.md and ONBOARDING.md"
```

---

### Task 5: `DESIGN.md`

**Files:**
- Modify: `DESIGN.md` (1,210 lines)

**Interfaces:** consumes the per-occurrence rule; produces nothing
consumed elsewhere.

- [ ] **Step 1: Read in full, apply the per-occurrence rule throughout**

This is the locked technical contract — almost entirely product-referring
("Aivyx" the system being described). Watch specifically for crate names
mentioned by name throughout (`aivyx-core`, `aivyx-capability`,
`aivyx-storage`, etc.) — these never change, per the Global Constraints.
Watch for any amendment-process language referencing "DESIGN.md" or
"PRODUCT.md" by filename — filenames are unaffected by this rename
(these docs keep their own filenames; only the prose *inside* them
changes).

- [ ] **Step 2: Commit**

```bash
git add DESIGN.md
git commit -m "docs: rename Aivyx to Aivyx PA in DESIGN.md"
```

---

### Task 6: `PRODUCT.md`

**Files:**
- Modify: `PRODUCT.md` (1,743 lines)

**Interfaces:** consumes the per-occurrence rule; produces nothing
consumed elsewhere.

- [ ] **Step 1: Read in full, apply the per-occurrence rule throughout**

The locked product contract (P1–P14). Almost entirely product-referring
("Aivyx is a self-learning, self-improving AI-personal assistant..." in
the one-line pitch, and throughout every Product Commitment). Same crate-
name and filename exclusions as Task 5. This is the longest single-file
task in this plan — budget real time for a careful, complete pass rather
than a rushed one; a partial rewrite here would be worse than an obvious
one, since it's the repo's own locked contract.

- [ ] **Step 2: Commit**

```bash
git add PRODUCT.md
git commit -m "docs: rename Aivyx to Aivyx PA in PRODUCT.md"
```

---

### Task 7: `docs/THREAT_MODEL.md`

**Files:**
- Modify: `docs/THREAT_MODEL.md` (627 lines)

**Interfaces:** consumes the per-occurrence rule; produces nothing
consumed elsewhere.

- [ ] **Step 1: Read in full, apply the per-occurrence rule throughout**

Operator-facing security posture doc — almost entirely product-referring
("Aivyx is a single-operator personal agent..."). This is a
security-critical document read by operators deciding whether to trust
the system; be precise, don't let a rushed word-swap introduce an
inaccurate claim.

- [ ] **Step 2: Commit**

```bash
git add docs/THREAT_MODEL.md
git commit -m "docs: rename Aivyx to Aivyx PA in THREAT_MODEL.md"
```

---

### Task 8: `docs/INSTALL.md`, part 1 (lines 1 through the confirmed split point)

**Files:**
- Modify: `docs/INSTALL.md` (only the first half, per Task 1's confirmed
  split point)

**Interfaces:**
- Consumes: `docs/superpowers/plans/rename-audit/install-split-point.md`
  (Task 1) for the exact line/heading to stop at.
- Produces: nothing consumed elsewhere. Task 9 covers the second half
  independently — the two tasks don't depend on each other's content,
  only on Task 1's shared split-point artifact.

- [ ] **Step 1: Read `install-split-point.md`, then read
  `docs/INSTALL.md` from the top through that confirmed line**

- [ ] **Step 2: Apply the per-occurrence rule throughout this half**

This is the install/setup instructions — expect a very high density of
literal shell commands (`./target/release/aivyx`, `aivyx init`, `aivyx
daemon install`, etc.) that need updating to `aivyx-pa` alongside the
prose. Update every `aivyx.toml` reference to `aivyx-pa.toml` and every
`~/.config/aivyx/`-shaped path to `~/.config/aivyx-pa/`. This half likely
covers the core install methods (shell installer, desktop app, Docker,
WSL2, manual build) — treat every command example as something a real
operator will copy-paste verbatim, so get the literal strings exactly
right, not just the surrounding prose.

- [ ] **Step 3: Commit**

```bash
git add docs/INSTALL.md
git commit -m "docs: rename Aivyx to Aivyx PA in INSTALL.md, part 1"
```

---

### Task 9: `docs/INSTALL.md`, part 2 (from the confirmed split point through the end)

**Files:**
- Modify: `docs/INSTALL.md` (the second half only)

**Interfaces:** consumes `install-split-point.md` (Task 1) for where to
start; produces nothing consumed elsewhere.

- [ ] **Step 1: Read `install-split-point.md`, then read
  `docs/INSTALL.md` from that confirmed line through the end of the
  file**

- [ ] **Step 2: Apply the per-occurrence rule throughout this half**

Same discipline as Task 8 — command examples get the same care as prose.
Coordinate mentally with Task 8's boundary (don't re-edit lines already
covered by the other task, don't leave a gap at the boundary either;
if the exact split line itself needs a change, whichever task's range
literally contains that line makes it — Task 8's range is "through the
confirmed line," Task 9's is "from the confirmed line," so the boundary
line itself belongs to Task 8).

- [ ] **Step 3: Commit**

```bash
git add docs/INSTALL.md
git commit -m "docs: rename Aivyx to Aivyx PA in INSTALL.md, part 2"
```

---

### Task 10: Tier-2 docs batch 1

**Correction note**: this plan originally specified four Tier-2 batch
tasks (10–13). Task 1's own real grounding found that two of the four
generated batches were **entirely** `docs/superpowers/` files (99 of the
original 195), which the operator then excluded from this rename per the
Global Constraints correction above. The two remaining batches — the
genuinely live Tier-2 docs — were renumbered `tier2-batch-1.md` and
`tier2-batch-2.md`. This plan now has two Tier-2 batch tasks, not four.

**Files:**
- Modify: every file listed in
  `docs/superpowers/plans/rename-audit/tier2-batch-1.md` (49 files)

**Interfaces:**
- Consumes: `tier2-batch-1.md` (Task 1, as corrected above).
- Produces: nothing consumed elsewhere.

- [ ] **Step 1: Read the batch's file list**

- [ ] **Step 2: For each file in the list, read it in full and apply the
  per-occurrence rule**

These are the repo's other living reference docs (e.g. `FEDERATION.md`,
`NONAGON.md`, `TOOLS.md`, and the rest of `docs/*.md` not already covered
by Tasks 3–9, and not excluded per the `docs/superpowers/` correction
above). Lower first-impression stakes than Tier 1, but still real
reference material an operator or contributor might read — apply the
same care, just without the same file-by-file individual task treatment.
Update every `aivyx.toml` mention to `aivyx-pa.toml` in the same pass.

- [ ] **Step 3: Self-verification pass**

After finishing the batch, re-run
`grep -n "\bAivyx\b" <each file in this batch>` and manually confirm
every remaining hit is a genuine org-reference you deliberately left
alone, not a missed product-reference. Note any genuinely ambiguous case
in your task report rather than silently picking one interpretation.

- [ ] **Step 4: Commit**

```bash
git add <files from this batch>
git commit -m "docs: rename Aivyx to Aivyx PA in Tier-2 docs batch 1"
```

---

### Task 11: Tier-2 docs batch 2

**Files:**
- Modify: every file listed in
  `docs/superpowers/plans/rename-audit/tier2-batch-2.md` (47 files)

**Interfaces:**
- Consumes: `tier2-batch-2.md` (Task 1, as corrected above).
- Produces: nothing consumed elsewhere.

- [ ] **Step 1: Read the batch's file list**

- [ ] **Step 2: For each file in the list, read it in full and apply the
  per-occurrence rule** (same discipline as Task 10 — see its own Step 2
  for the full description; this task is structurally identical, just a
  disjoint file set)

- [ ] **Step 3: Self-verification pass** (same as Task 10's own Step 3)

- [ ] **Step 4: Commit**

```bash
git add <files from this batch>
git commit -m "docs: rename Aivyx to Aivyx PA in Tier-2 docs batch 2"
```

---

## After all tasks

Once every task above is merged and the full workspace builds/tests
clean:

1. **The GitHub repo rename and local directory rename happen as one
   manual, controller-driven step** — not a plan task. `gh repo rename
   aivyx-pa` (or the GitHub web UI) on `Aivyx-Agent/aivyx`, then rename
   the local directory `~/Projects/Rust/aivyx/` →
   `~/Projects/Rust/aivyx-pa/`. Confirm the old clone/web URLs still
   redirect (GitHub's standard behavior) before considering this done.
2. Update `~/Projects/Rust/CLAUDE.md`'s own directory listing and the
   `aivyx-ecosystem` repo's README/ROADMAP/GLOSSARY, plus
   `aivyx-coder`'s disambiguation text — this is **sub-project B**, its
   own separate spec, not part of this plan.
3. `aivyx-brand`/`aivyx-website` — **sub-project C**, its own separate
   spec.

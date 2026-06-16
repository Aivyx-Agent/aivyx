# Phase 66 — Starter Profile Templates

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Ship starter profile templates and an `aivyx init --template
<name>` flag so a fresh operator gets from "downloaded the
binary" to "useful agent" in under five minutes. Three
templates ship in this phase: `coder`, `researcher`,
`personal`. Templates live both bundled (via `include_str!`)
and in an optional user directory override at
`~/.local/share/aivyx/templates/`. The existing interactive
wizard gains a pre-fill path that reads defaults from the
selected template; the operator still steps through each
prompt but the suggested values come from the template.

After Phase 66 the onboarding flow is:

```sh
aivyx init --list-templates           # discover available templates
aivyx init --template coder           # interactive, pre-filled from `coder`
aivyx init --template researcher      # …or another archetype
aivyx                                  # run the agent
```

The substrate is the load-bearing piece. Templates are TOML
fixtures; adding more in future phases is cheap once the
registry + wizard-integration plumbing exists.

## Why now

1. **Largest remaining adoption-shape gap.** Post-Phase-65 the
   identity-transfer story is complete and the substrate is
   substantial. The friction now is "I have the binary; what
   do I write in `aivyx.toml`?". The existing `examples/aivyx.toml`
   is a feature-demonstration file, not a usable starter.
2. **Operator-feedback-shaped per the original codebase
   review.** Starter profile bundles were one of the three
   directions named in the post-Phase-60 review; the other
   two (Distribution → Phases 61, Reach → Phases 62–63, plus
   the Identity arc Phases 64–65) have all landed.
3. **Compositional with the existing wizard.** Phase 44's
   `aivyx init` already runs through provider / model / paths
   / Profile prompts. Pre-filling from a template integrates
   cleanly without rewriting the wizard.
4. **Q-block fully resolved at design time.** Bundled +
   user-dir hybrid (Q1), three templates (Q2), pre-fill
   wizard (Q3), both discovery modes (Q4).

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 66 adds CLI flags, a
  template registry, three TOML fixtures, and wizard
  integration. No D-deliverable reshape. Prediction: streak
  **extends to thirteen** consecutive phases (currently at 12).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment text changes;
  no Delivery Status refresh. The starter-templates milestone
  is operator-feedback-shaped, not a P1–P14 commitment.
  Prediction: streak **extends to six** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Every Phase 66 surface lives in
  `crates/aivyx-channel/src/bin/aivyx_modules/init.rs`,
  new `init_templates.rs` (or similar) module, three new
  `examples/templates/*.toml` files. No path touches
  `aivyx-core`. Prediction: streak **extends to fourteen**
  consecutive phases (new record, beating Phase 65's 13).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_66.md scaffold

This file. Update `docs/README.md` to show Phase 66 as Open.

### Task 2 — Template registry substrate

New module `crates/aivyx-channel/src/bin/aivyx_modules/init_templates.rs`:

- `Template { name: &'static str, description: &'static str,
  toml_content: &'static str }`
- `bundled_templates() -> &'static [Template]` — returns the
  embedded templates via `include_str!`.
- `load_template(name) -> Result<Template, String>` — looks up
  by name. User-dir override first
  (`~/.local/share/aivyx/templates/<name>.toml`), then bundled.
  User-dir takes precedence per Q1(a).
- `list_templates() -> Vec<TemplateMeta>` — unions user-dir +
  bundled, dedups by name (user wins), returns `(name,
  description, source)` triples for display.

User-dir templates use the same TOML shape as bundled. The
description is read from a leading TOML comment line of the
form `# description: …` so user-authored templates can self-
describe without a separate metadata file.

### Task 3 — CLI flag parsing

`init` subcommand parser extends:

- `aivyx init --template <name>` — pre-fill wizard from the
  named template.
- `aivyx init --template` (no name) — print the template list
  + exit 0.
- `aivyx init --list-templates` — explicit list-mode flag.

Both `--template` (no name) and `--list-templates` route to
the same list-printing path per Q4(c) (both work). The list
output shows `name`, `description`, and `(bundled)` or
`(user)` source tags.

### Task 4 — Wizard pre-fill integration

Phase 44's `aivyx init` wizard runs through several prompts
(provider, model, paths, profile name, primary use case,
communication style). Each prompt becomes "if a template is
selected and contains this field, use its value as the
default; otherwise use the existing default."

The wizard still walks the operator through each prompt —
this isn't a non-interactive bypass. The template pre-fills
the suggestions so the operator can press Enter through fields
they're happy with.

Templates declare their starter values as a TOML doc
(`[profile]`, `[[role]]`, `[anthropic]` etc.). The wizard
parser reads selected fields:

- `assistant_name` → assistant-name prompt default.
- `primary_use_cases[0]` → primary-use-case prompt default.
- `communication_style` → communication-style prompt default.
- `[agent] provider` → provider-choice default.
- `[agent] model` → model-name default.

Other template fields (role declarations, scope lists, MCP
servers) are merged into the generated TOML when the wizard
writes the final file. The wizard's existing render path is
extended to honor template-supplied role declarations.

### Task 5 — `coder` template

`examples/templates/aivyx-coder.toml`. Software engineering
focus:

- Provider: Ollama default (codellama or llama3 family);
  Anthropic Claude commented as alternative.
- Profile: `assistant_name = "Codex"` (placeholder; operator
  can override), `primary_use_cases = ["software engineering"]`,
  `behavioral_preferences = ["prefer integration tests over
  mocks", "always cite source files when referencing code"]`,
  `behavioral_constraints = ["never autonomously commit code",
  "always confirm destructive shell commands"]`.
- One main role with capability_scopes covering
  `fs.read`, `fs.write`, `shell.exec`, `memory.read/write`,
  `web.fetch` (for crate docs lookup).
- MCP web-search enabled by default.
- Comments explain the security tier model and where to tweak
  for a real deployment.

### Task 6 — `researcher` template

`examples/templates/aivyx-researcher.toml`. Research / synthesis
focus:

- Provider: same options.
- Profile: `assistant_name = "Inquiry"` (placeholder),
  `primary_use_cases = ["research and synthesis", "literature
  review"]`, `behavioral_preferences = ["always cite sources",
  "summarize before quoting"]`, `behavioral_constraints =
  ["never edit files without operator confirmation"]`.
- One main role: heavy `web.fetch`, `memory.read/write`,
  `fs.read` (no shell, no fs.write by default).
- MCP web-search enabled prominently.

### Task 7 — `personal` template

`examples/templates/aivyx-personal.toml`. Personal-assistant
focus:

- Provider: same options.
- Profile: `assistant_name = "Mira"` (placeholder),
  `primary_use_cases = ["personal task management", "daily
  briefings"]`, `behavioral_preferences = ["conclusion-first
  paragraphs", "three-bullet lists when summarizing"]`,
  `behavioral_constraints = ["never share sensitive context
  externally without asking"]`.
- Role with capability_scopes: memory-heavy, `fs.read/write`
  on a notes-style sandbox, `web.fetch`, `notify.send`.
- Commented-out starter `[[schedule]]` for "morning briefing"
  and `[[notify_target]]` for Telegram (operator fills in
  chat_id).

### Task 8 — Tests

- Template registry unit tests: bundled list non-empty, user-
  dir override picks user template, list dedup correctness.
- Description-extraction parser: reads `# description: …` from
  TOML leading comment.
- Wizard pre-fill: given a template, the wizard's effective
  defaults match the template's declared values for
  assistant_name / primary_use_case / communication_style /
  provider / model.
- TOML validity: every bundled template parses via
  `aivyx-config` without errors (catches malformed templates
  at test time, not at user time).

### Task 9 — Docs

- `docs/INSTALL.md` "First-run checklist" updates to mention
  `--template` and `--list-templates`.
- New `docs/TEMPLATES.md` describing what each starter looks
  like, when to pick which, and how to author a custom
  user-dir template.
- README quickstart mentions `aivyx init --template <name>`
  as the fast path.

### Task 10 — Exit commit

- `ROADMAP.md` Phase 66 frozen entry.
- `docs/PRODUCT_ROADMAP.md` updates: new "Onboarding Templates"
  milestone or extension of existing onboarding section.
- `docs/README.md` status table flipped to Frozen with backfill.
- Prediction-vs-reality block filled.
- Exit-criteria block completed.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Location:** (c) Hybrid. Bundled templates embedded
  via `include_str!`; user-dir override at
  `~/.local/share/aivyx/templates/`. User wins on name
  conflict.
- **Q2 — Count:** (c) Three templates: `coder`, `researcher`,
  `personal`.
- **Q3 — Wizard interaction:** (b) Pre-fill wizard prompts
  from template. The operator still walks each prompt; the
  template just sets the suggested defaults.
- **Q4 — Discovery:** (c) Both: `--list-templates` and
  `--template` (no name) both print the list.

## Deferrals

**Rolling deferrals carried into Phase 66:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `AuditEventKind::AutoNotifyDispatched` (Phase 63).
- All Phase 62 / 63 reach-axis deferrals.
- Phase 64 deferrals (encrypted export format, selective
  import, profile diff display, Web UI export/import, multi-
  source merge, schema migration tooling).
- Phase 65 deferrals (atomic-tx import, profile auto-import,
  merge-strategy imports, selective imports, force-flag
  scoping).

**Phase 66 deferrals (recorded at exit):**

- More templates beyond the initial three (e.g.
  `data-analyst`, `writer`, `student`). The substrate
  supports arbitrary additions; future phases or community
  contributions add them as use cases surface.
- Template parameter substitution (`{{operator_name}}`
  placeholders prompted at init time). Phase 66 ships literal
  defaults; wizard prompts let operators override per-field.
- Template tagging / search ("list all templates that include
  `notify.send`").
- Web UI surface for template selection. CLI-only in v1.
- Versioned template registry (multiple template versions
  available; operator picks one).
- Sharing templates between operators (curl-from-URL,
  community registry). Today operators copy `.toml` files
  manually into `~/.local/share/aivyx/templates/`.
- Schema migration for template format changes (Phase 66
  ships v1; future revisions need migration paths).

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak **extends to thirteen**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Phase 66 shipped operator-facing substrate (CLI flags,
  template registry, wizard pre-fill, three template fixtures)
  with no D-deliverable reshape.

- **PRODUCT.md** — Predicted: streak **extends to six**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Starter templates are operator-feedback-shaped substrate,
  not a P1–P14 commitment; no Delivery Status refresh.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to fourteen** (new record). **Reality:
  correct.** Hash unchanged at entry and exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Every Phase 66 surface routed through
  `crates/aivyx-channel/src/bin/aivyx_modules/init_templates.rs`,
  `init.rs`, `aivyx.rs` and the three `examples/templates/*.toml`
  files. Fourteen consecutive phases — longest production-core
  run in project history; beats Phase 65's 13.

- **Test count** — Predicted: positive (~+20–30). **Reality:
  +18** (1176 → 1194). Slight undershoot from prediction;
  the substrate is heavy in template TOML content (which
  tests once for parse validity) and tooling/wizard plumbing
  (which has integration coverage via parse_cli_args_from
  tests + the registry's listing tests).

- **New workspace deps** — Predicted: zero. **Reality:
  correct.** All new code reuses `toml_edit` (existing
  Phase 58 dep), `std::fs`, `serde_json` etc. — no Cargo.toml
  changes.

- **Scope ambition** — Operator chose the more ambitious
  options across the Q-block (hybrid location, three
  templates, pre-fill wizard, both discovery modes). The
  ambitious choices delivered cleanly — no scope adjustments
  at implementation time, all four sign-offs held.

## Exit criteria

- [x] Template registry substrate (bundled + user-dir
  override + listing + lookup) — Task 2, commit `ccfdbcd`.
- [x] CLI flags `--template <name>`, `--template` (no name),
  `--list-templates` — Task 3, commit `ccfdbcd`.
- [x] Wizard pre-fill integration via `TemplateDefaults` +
  `render_with_template` splice-back — Task 4, commit
  `ccfdbcd`.
- [x] `coder` template — Task 5, commit `ccfdbcd`.
- [x] `researcher` template — Task 6, commit `ccfdbcd`.
- [x] `personal` template — Task 7, commit `ccfdbcd`.
- [x] 14 registry tests + 6 parser tests = 20 unit tests
  across registry, parser, and pre-fill flow — Task 8,
  commit `ccfdbcd`. Every bundled template tested for TOML
  validity at test time.
- [x] `docs/INSTALL.md` First-run checklist updated; new
  `docs/TEMPLATES.md` describing each starter + custom-
  template path; README quickstart points at the template
  fast-path — Task 9, commit `c5b25eb`.
- [x] ROADMAP.md + PRODUCT_ROADMAP.md + docs/README.md
  refreshed — Task 10 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (Q1(c) hybrid, Q2(c) three templates,
  Q3(b) pre-fill wizard, Q4(c) both discovery modes).
- [x] DESIGN.md streak extends to thirteen.
- [x] PRODUCT.md streak extends to six.
- [x] Production-core streak extends to fourteen (new record).
- [x] Test count delta: +18 (1176 → 1194).
- [x] Zero clippy warnings.
- [x] Prediction-vs-reality block filled.

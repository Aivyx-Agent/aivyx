# Aivyx → Aivyx PA rename: design (Sub-project A — the core rename)

**Status:** approved, ready for implementation planning
**Repo:** `aivyx` (soon `aivyx-pa`) — this spec covers only this repo

## Motivation

Every sibling repo in the ecosystem (`aivyx-coder`, `aivyx-broker`,
`aivyx-yubi`, `aivyx-kvcache`, `aivyx-confine`, `aivyx-checkpoint`,
`aivyx-recall`, `aivyx-injection-guard`, `aivyx-brand`, `aivyx-website`,
`aivyx-wallpapers`) was *named* `aivyx-<noun>` from creation. `aivyx` itself
predates that convention — it's the thing the convention was named after,
not a straggler that missed it. Beyond the naming asymmetry, "Aivyx" today
does double duty: it's both the org/ecosystem name (the `Aivyx-Agent`
GitHub org, `aivyx-brand`, `aivyx-website`, the trademark holder identity)
*and* the specific flagship product's name. This project resolves that
overlap and brings the flagship's technical identity in line with every
sibling repo.

## The naming split (locked)

- **"Aivyx"** stays the org/ecosystem name: `Aivyx-Agent` (GitHub org),
  `aivyx-brand`, `aivyx-website`, the trademark holder identity, and how
  the ecosystem as a whole is referred to. Sibling repos' own identities
  (`aivyx-coder`, `aivyx-broker`, etc.) are unaffected by this project.
- **"Aivyx PA"** becomes the flagship product's own name everywhere it is
  currently used as *the product* — "Aivyx PA is a self-learning agentic
  personal assistant," the `TRADEMARK.md`/`COMMERCIAL.md` subject, the
  GitHub repo, the local directory.
- Casing convention: **"Aivyx PA"** (space, capital PA) in prose,
  marketing, and documentation; **`aivyx-pa`** (all-lowercase, kebab-case)
  for the repo name, the installed binary, and filesystem paths — the
  same relationship every technical identifier in this ecosystem already
  has to its spoken name.

## Scope

**In scope (this spec, sub-project A):**
1. GitHub repo rename: `Aivyx-Agent/aivyx` → `Aivyx-Agent/aivyx-pa`.
2. Local directory rename: `~/Projects/Rust/aivyx/` →
   `~/Projects/Rust/aivyx-pa/`.
3. Installed binary rename: `crates/aivyx-cli/Cargo.toml`'s
   `[[bin]] name = "aivyx"` → `"aivyx-pa"`.
4. Config/data/state path rename: `~/.config/aivyx/` →
   `~/.config/aivyx-pa/`, and the equivalent `~/.local/share/`/
   `~/.local/state/` paths.
5. Every other embedded string constant carrying the literal product
   identity — at minimum the confirmed real one,
   `aivyx-channel/src/keyring_store.rs`'s `const SERVICE: &str = "aivyx"`
   (the OS keyring service name), plus a full audit at plan time for
   others (the daemon socket path, any systemd/launchd service unit name,
   log file paths, the `AIVYX_*` env var prefix if one exists) — this
   spec commits to the audit happening, not to a pre-enumerated exhaustive
   list, since the real list can only be confirmed by grepping the real
   code at implementation time.
6. Product-name prose rewrite across all **192 live docs files** (every
   `.md` file in the repo outside `docs/archive/`), per the per-occurrence
   rule below, tiered by priority (see "Docs-rewrite approach").
7. `TRADEMARK.md` extended to protect both "Aivyx" and "Aivyx PA" as
   names (the safer, more protective reading — the mark should cover the
   name someone would actually try to copy, not just the org-level name).

**Explicitly out of scope:**
- The 34 internal Cargo crate names (`aivyx-core`, `aivyx-capability`,
  `aivyx-storage`, etc.) — stay exactly as they are. Nobody outside the
  project sees these; renaming them is pure churn with real risk of
  breaking something for zero user-facing benefit.
- Any auto-migration code or compat shim for existing installs — this is
  a clean break, documented only (see "Existing-install migration"
  below), not built.
- Cross-repo ripple: `aivyx-coder`'s disambiguation text, `aivyx-ecosystem`'s
  README/ROADMAP/GLOSSARY, root `~/Projects/Rust/CLAUDE.md`,
  `aivyx-broker`'s/`aivyx-yubi`'s own docs referencing the flagship —
  **deferred to sub-project B**, its own spec, once this project's real
  new identity exists to reference.
- `aivyx-brand`'s marketing copy and `aivyx-website`'s content — **deferred
  to sub-project C**, its own spec.
- The actual trademark filing/legal action, if "Aivyx" is a real
  registered mark beyond the `TRADEMARK.md` notice — outside anything a
  code/doc change can execute. This spec updates the documentation
  describing trademark protection; it does not constitute or execute any
  real legal/trademark-office action. That remains the operator's own
  responsibility, separate from this project.
- `docs/archive/phases/PHASE_N.md` (212 files) — frozen by this repo's
  own existing convention (editable only via `docs(phase-N):`-prefixed
  commits), and they accurately record what the product was called *at
  the time each phase shipped*. Rewriting "Aivyx" to "Aivyx PA"
  retroactively in them would be revising history, not fixing a stale
  reference. Untouched by this project.

## Existing-install migration

Clean break, documented only — no auto-migration code, no compat shim
binary. `CHANGELOG.md`'s entry for the release that ships this rename
states plainly: this is a breaking rename, the binary is now
`aivyx-pa` not `aivyx`, and anyone with an existing
`~/.config/aivyx/` who wants to keep their data should manually move it
to `~/.config/aivyx-pa/` (and the equivalent `~/.local/share/`/
`~/.local/state/` paths) before or after upgrading. Chosen deliberately
given this is a pre-1.0 product with no known wide install base — the
cost of building and maintaining real migration machinery isn't justified
by the actual blast radius.

## Docs-rewrite approach

192 live files is too large to treat as a mechanical find-and-replace —
"Aivyx" genuinely means two different things depending on context, and
getting it wrong in either direction (leaving the product un-renamed, or
accidentally renaming the org/ecosystem) is worse than leaving it alone.

**The rule for every occurrence:** refers to the specific product,
software, daemon, or binary → becomes "Aivyx PA." Refers to the ecosystem,
the org, the GitHub org, or brand language in the abstract → stays
"Aivyx."

**Tiering** (not a hard gate — a priority order for care and sequencing):
- **Tier 1** (do first, most carefully — the front door for anyone
  evaluating or setting this up): `README.md`, `VISION.md`, `PRODUCT.md`,
  `DESIGN.md`, `TRADEMARK.md`, `COMMERCIAL.md`, `CHANGELOG.md`,
  `docs/THREAT_MODEL.md`, `docs/INSTALL.md`, `docs/ONBOARDING.md`. Almost
  entirely product-referring today.
- **Tier 2**: the remaining living `docs/*.md` reference docs (roughly
  180 files — `FEDERATION.md`, `NONAGON.md`, `TOOLS.md`, and the rest).
  Same rule, lower first-impression stakes, but still real user-facing
  reference material.
- **Excluded**: `docs/archive/phases/*.md` (212 files), per the
  historical-accuracy principle above.

**Verification, honestly scoped:** there is no fully mechanical pass/fail
check here, since "Aivyx" legitimately survives in many places (org
references). The practical verification is a self-review pass: after each
tier, `grep -rln "\bAivyx\b" <tier files> | grep -v archive/` to
re-surface every remaining bare "Aivyx" occurrence and manually confirm
each one is a genuine org-reference, not a missed product-reference. This
is a checklist to walk, not an automated gate.

## Testing & verification (technical rename)

- `cargo build --workspace` after the `[[bin]]` rename must produce a
  binary literally named `aivyx-pa`, with zero other build changes (no
  crate renamed, so the existing dependency graph and all existing tests
  are otherwise untouched).
- `cargo test --workspace` must still pass unchanged — internal crate
  names didn't move, so no test should need editing for the rename
  itself (tests that assert on the literal string `"aivyx"` in a
  path/config/keyring-service context are the one real exception; the
  plan-time audit from scope item 5 should surface these).
- Manual confirmation that a fresh `aivyx-pa init` writes to
  `~/.config/aivyx-pa/` and `~/.local/share/aivyx-pa/`, not the old
  paths — this is the one behavior change a user would actually observe
  at runtime, so it's worth confirming directly rather than only via
  code review.

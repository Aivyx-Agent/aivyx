# Licensing & Commercial Model (Chapter Charter)

> **Status:** 🧭 **design contract — CR.0 (not yet executed).** This document is
> the locked reference for moving Aivyx from **MIT** to the **Business Source
> License 1.1 (BUSL-1.1)** going forward: **free for personal / individual /
> non-commercial use, a paid commercial license for any business or production
> use,** auto-converting back to MIT after a fixed term. Nothing in the tree has
> changed yet — the LICENSE swap, the dependency audit, the commercial path, and
> the contributor terms are the phases below (CR.1–CR.6). Decisions locked by the
> operator: (1) **BSL**, not FSL/AGPL — the gate is *commercial vs. personal*,
> not *competing vs. not* and not *SaaS vs. internal*; (2) the **whole public
> repo** moves (engine + public tool crates), with verticals staying private as
> already planned.

## 1. The decision and why it isn't MIT anymore

Aivyx shipped public + MIT (Chapter Q, v0.1.0 / v0.2.0). MIT is maximally
permissive: anyone — including a competitor or any for-profit company — may use,
modify, host, and **sell** the software with no obligation back to the author.
That is the right posture for adoption and trust, but it leaves **no monetization
hook on the core itself**: the open-core plan ([[aivyx-ecosystem-roadmap]])
monetizes only the *private verticals* and *hosting*, never the engine.

The operator's intent is narrower and clearer: **the public end user runs Aivyx
for free; a commercial end user pays.** That is a *commercial-vs-personal* gate.
The license that expresses exactly that gate is the **Business Source License
1.1** with an Additional Use Grant scoped to personal / non-commercial use.

### Why not the alternatives (recorded so we don't relitigate)

| License | Gates on | Verdict for "free personal / paid commercial" |
|---|---|---|
| **MIT** (today) | nothing | ❌ no monetization hook on the core |
| **AGPL-3.0 + commercial** | *offering it as a network service* | ❌ a company using it internally pays nothing |
| **FSL** | *competing commercial use* | ❌ internal commercial use is free; only resellers pay |
| **BSL 1.1 + personal-use grant** | *commercial / production use* | ✅ **exactly the intended split** |

The cost of BSL is **honesty about the label**: BSL is **"source-available," not
OSI-approved "open source."** Every doc that today says "open source" must be
corrected to "source-available" (CR.5). This is non-negotiable for the trust
story — a privacy-first agent cannot afford a misleading license claim.

## 2. What cannot be undone — and what that means

**v0.2.0 and every prior commit are MIT in perpetuity.** A license is granted at
the moment of distribution; it cannot be revoked. Anyone may fork the v0.2.0
baseline and do anything MIT allows, forever. **The relicense therefore applies
only from the next release forward** (the first BSL tag — see CR.6).

This is not a problem, it is how every comparable relicense worked (HashiCorp,
Sentry, Redis, MariaDB, CockroachDB): the free-rider's fork is frozen at an old,
unmaintained snapshot while the maintained line moves ahead under BSL. **Our moat
is velocity + brand + verticals + hosting, never the frozen snapshot.**

## 3. Standing on the right to relicense

Relicensing requires holding the rights to **all** the code being relicensed.

- **Today:** the operator is the sole author — full freedom to relicense.
- **The moment outside contributions are accepted, that breaks.** A contributor's
  patch is theirs under the inbound license; without an explicit grant we could
  not relicense their lines, nor sell a commercial license covering them. So
  **contributor terms (a DCO or CLA) that grant relicensing rights are a
  prerequisite for accepting any external PR** — handled in CR.4 *before* the repo
  invites contributions.

## 4. The BSL parameters (the contract)

The BSL 1.1 template has four fill-in parameters. These are the locked values:

- **Licensor:** Julian (Aivyx) / the Aivyx-Agent project.
- **Licensed Work:** Aivyx, the first BSL-tagged version onward (CR.6).
- **Additional Use Grant:** *personal and non-commercial use.* Draft wording
  (final text lands in CR.2, reviewed against the official template):

  > You may use, copy, modify, and create derivative works of the Licensed Work
  > for **personal, individual, educational, research, evaluation, and other
  > non-commercial purposes**. "Non-commercial" means use that is **not primarily
  > intended for or directed toward commercial advantage or monetary
  > compensation**, including use by an individual for personal projects and use
  > by a registered non-profit or accredited educational institution. **Any other
  > use — including any use by or on behalf of a for-profit entity, any use in
  > production in connection with a commercial product or service, and any use
  > that generates revenue — requires a commercial license from the Licensor.**

- **Change Date:** four (4) years after the publication date of **each** released
  version (every release carries its own clock).
- **Change License:** **MIT** (continuity with Aivyx's origin; on the Change Date
  that version reverts to the exact MIT terms it shipped under before).

**SPDX note (load-bearing for tooling):** the SPDX identifier is **`BUSL-1.1`**
(not "BSL-1.1"). Cargo's `license` field must use `BUSL-1.1` so `cargo`,
`cargo-deny`, and downstream scanners parse it. Where the personal-use grant makes
the expression non-standard, fall back to `license-file` pointing at `LICENSE`.

## 5. Scope — what moves and what was always private

- **Moves to BSL:** the entire **public** workspace — the engine crates (turn
  loop, `aivyx-capability`, audit chain, `aivyx-team`/Nonagon, memory, persona
  governance, IPC, TUI, web Studio) **and** the public tool-process crates
  (`aivyx-gmail`, `aivyx-calendar`, `aivyx-drive`, `aivyx-contacts`,
  `aivyx-toolkit`, etc.). One license across the public repo keeps it simple and
  defensible; a per-crate split buys nothing here.
- **Was always private (unchanged):** commercial verticals (Kitchen/Factory
  toolkits, customised Nonagon `TeamConfig`s), hosted Harbor, federation. The BSL
  on the public core is a *new* monetization layer *in addition to* these.
- **The "Aivyx" name is trademark, not copyright** — a BSL relicense does not
  protect the brand. Trademark posture is out of scope for this chapter (noted so
  it isn't assumed covered).

## 6. Phase plan (docs-first, small phases per project convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **CR.0** | **This design contract** | locked reference; status banner flips per phase |
| **CR.1** | **Dependency license audit** | `cargo-deny`/`cargo-license` over the tree; confirm **no copyleft (GPL/AGPL) dep** forces the whole work open and that BSL-on-our-code conflicts with nothing. Gate: must pass before CR.2. |
| **CR.2** | **The LICENSE swap** | `LICENSE` → filled BUSL-1.1 text; preserve the MIT text as `LICENSES/MIT.txt` (the Change License + historical form); workspace `Cargo.toml` `license = "BUSL-1.1"` (or `license-file`); update any per-crate `license.workspace` consumers; add the standard BSL header note. |
| **CR.3** | **Commercial-license path** | `COMMERCIAL.md`: precisely *what* needs a license (the §4 grant in plain English), *how* to obtain one (contact / email), and a pricing placeholder. The thing a commercial user lands on. |
| **CR.4** | **Contributor terms** | `CONTRIBUTING.md` + a **DCO** (lightweight) or **CLA** (stronger) granting relicensing/commercial-sublicensing rights. **Must precede any external PR** (§3). |
| **CR.5** | **Positioning & docs refresh** | Correct every "open source" → "source-available" (README, INSTALL, DESIGN, ROADMAP, this repo's description); add a licensing FAQ ("can I use it at work?", "what counts as commercial?", "when does it become MIT?"); cross-link COMMERCIAL.md. |
| **CR.6** | **First BSL release** | Tag the first version under BSL (**v0.3.0** or **v1.0** — operator's call at the time); cargo-dist release notes lead with the license change; record the green release run. |

**Discipline:** CR.1 is a hard gate — if a dependency's license is incompatible
with shipping the combined work under BSL, that's a blocker to resolve (swap the
dep or carve it out) *before* the LICENSE swap, not after.

## 7. Open questions to resolve in-phase (not blockers to CR.0)

- **Change Date granularity** — per-release 4-year clocks (chosen above) vs. a
  single global date. Per-release is the MariaDB/Sentry norm and is assumed.
- **DCO vs. CLA** (CR.4) — DCO is frictionless but a weaker rights grant; a CLA is
  stronger but adds contributor friction. Decide when contributions become real.
- **First BSL version number** (CR.6) — v0.3.0 (incremental) vs. v1.0 (signals the
  model is set). Operator's call at release time.
- **crates.io** — not currently a distribution channel (releases are cargo-dist
  binaries + GHCR), so its OSI-license preference doesn't bind us today; revisit
  only if/when publishing crates.

---

*Chapter Charter is the governance/licensing chapter: it changes the terms under
which Aivyx is offered, not the code's behavior. It is the monetization hook the
open-core roadmap ([[aivyx-ecosystem-roadmap]]) was missing on the core itself.*

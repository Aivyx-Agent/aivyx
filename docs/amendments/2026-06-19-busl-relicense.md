# Amendment A14 — Relicense: Open-Core MIT → BUSL-1.1

**Date:** 2026-06-19
**Chapter:** Charter (CR.0–CR.6)
**Supersedes:** The licensing *mechanism* in **DESIGN.md → Deliverable 2 — The
Open-Core Line (LOCKED 2026-04-13)**, specifically "The Rule," the v1 license
table, and the per-crate "License" column. The *spirit* of Deliverable 2 (a free
core + a paid commercial side) is preserved and strengthened; only the instrument
changes. PRODUCT.md is unaffected (it carries no licensing clause).
**Implementing phase:** Chapter Charter, CR.5 (the full chapter is CR.0–CR.6).
**Reference contract:** [`docs/LICENSING.md`](../LICENSING.md).

---

## What changed

Deliverable 2 locked an **open-core** model: the protocol + security-surface
crates ship **MIT**, and money is made on *separate* commercial products built on
top (`aivyx-engine`, `aivyx-hub`, hosted services). The public core itself had
**no monetization hook** — anyone, including a for-profit, could use and sell it
for free.

Chapter Charter changes the instrument. The **entire public workspace** — every
crate Deliverable 2 listed as MIT, engine and public tool crates alike —
relicenses to the **Business Source License 1.1 (BUSL-1.1)**:

- **Free** for personal, individual, non-commercial, educational, and research
  use (the Additional Use Grant).
- **Paid** commercial license for any business / production / revenue use.
- **Auto-reverts to MIT** four years after each version is published (per-release
  clock), via the BUSL Change License.

So Deliverable 2's per-crate "MIT" entries are now read as **"BUSL-1.1 (→ MIT
after 4 years)."** The earlier MIT-tagged crates that already shipped (v0.2.0 and
prior) **remain MIT in perpetuity** — a license can't be revoked; the relicense
applies only from the first BSL-tagged release forward (CR.6).

## Why this is a strengthening, not a reversal, of the open-core principle

Deliverable 2's intent — *the protocol and security surface stay auditable; the
business is sustainable* — is fully preserved:

1. **Auditability is untouched.** BUSL-1.1 is **source-available**: the entire
   tree stays readable, forkable for non-commercial use, and reverts to MIT.
   "Security surface must be readable to be trustable" still holds.
2. **The open-core split gains the hook it lacked.** Deliverable 2 monetized only
   *future separate products*. Charter adds a monetization layer **on the core
   itself** (commercial use pays) **in addition to** the still-private verticals
   and hosting — closing the gap [[aivyx-ecosystem-roadmap]] identified.
3. **The MIT origin is honored.** The Change License is **MIT** (GPL-compatible,
   satisfying BUSL Covenant #1), so every version eventually lands exactly where
   Deliverable 2 put it.

## What this amendment deliberately does not change

- **Not "open source" anymore — "source-available."** BUSL-1.1 is **not** an
  OSI-approved open-source license. Every doc claim of "open source" / "MIT" for
  the Aivyx code is corrected to "source-available under BUSL-1.1" (CR.5). This
  honesty is load-bearing for the privacy/trust story.
- **Trademark posture is unchanged.** The "Aivyx" name/logo remain trademarked
  ([`TRADEMARK.md`](../../TRADEMARK.md)); a code license (free or commercial)
  grants no brand rights. Deliverable 2's `aivyx-pa` "MIT + branded" exception
  becomes "BUSL-1.1 + branded" — the *branding* rule is identical.
- **The deferred commercial components stay commercial.** `aivyx-engine`,
  `aivyx-hub`, federation, hosted Harbor, and the private verticals were always
  commercial and remain so; BUSL on the public core is a new layer beside them.

## Why now

Chapter Charter is the governance/licensing chapter (CR.0 design contract
2026-06-19 batch). CR.2 swapped `LICENSE` to BUSL-1.1, CR.3 added `COMMERCIAL.md`,
CR.4 added the CLA gate. CR.5 refreshes positioning — and a contract-level
statement (Deliverable 2) cannot be corrected by docs alone; it requires this
amendment so DESIGN.md stays self-consistent with the shipped `LICENSE`.

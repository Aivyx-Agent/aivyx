# v0.9 — the Interface Polish phase (plan of record, locked 2026-07-04)

> v0.8.0 marked core refinement converged. v0.9 makes every
> operator-facing surface (the Studio's 15 screens, the TUI, the desktop
> shell) feel *finished*, ships the two locked v1.0-runway
> infrastructure chapters, and strands nothing in `/classic`. Work ships
> continuously as v0.8.x releases; **v0.9.0 cuts as the capstone** when
> the polish backlog is empty. UI stack is locked: Dioxus + Stitch —
> refinement, not rewrite (see the UI-stack decision of 2026-07-04).

## Working mode

This phase inverts the dogfood loop: GUI rendering can't be verified
from the build sandbox (no GPU, no live serve), so **the operator drives
the Studio in a real browser and narrates friction; the assistant fixes
and redeploys**. The Vitrine walkthrough (below) is the structured form
of that loop and produces the phase's backlog.

## Sequence

**Table audited 2026-08-27** — this table had not been touched since
2026-07-04 (the day it was locked, before Vitrine even started), so rows
3 and 5 were silently stale by 7+ weeks: both had actually shipped, but
nothing recorded it here. Corrected against real evidence (`docs/
VITRINE.md`, `CHANGELOG.md`, a live `/classic` nav check), not inferred.
**Rows 4, 6, 7, and 8 are now fully decomposed and sequenced in
[`docs/POLISH_WAVES.md`](POLISH_WAVES.md)** — per this table's own
maintenance convention ("if an entry grows task lists or open
questions, it has outgrown the roadmap and belongs in its own doc"),
their prose is no longer duplicated here.

**All 8 rows done as of 2026-09-03 — this file's own stated capstone
rule ("v0.9.0 cuts as the capstone when the polish backlog is empty")
is now satisfied.** Row 4 (`docs/POLISH_WAVES.md`'s 8 sub-projects) was
the last to close. Cutting the actual `v0.9.0` release (bumping the
workspace version from `0.8.3`, converting `CHANGELOG.md`'s
`[Unreleased]` section into a dated `[0.9.0]` entry, tagging) is a
separate, deliberate release-engineering step this plan doesn't
prescribe the timing of — not done automatically just because the
backlog emptied.

| # | Chapter | Scope | Status |
|---|---|---|---|
| 1 | **Gatehouse** | Studio remote auth. Found on scoping: Chapter Postern had already built the auth *mechanism* (`web_ui_auth_token`: Bearer/Basic/cookie, constant-time, `/ws`-gated) — Gatehouse added the refuse-to-bind interlock at config load, the `web_ui_insecure_no_auth` escape hatch, and first-boot token generation in the Harbor appliance. See docs/GATEHOUSE.md. | ✅ done (7ba97b2) |
| 2 | **Freight** | Signed pack bundles, complete: format core (aivyx-pack), the `aivyx pack` CLI, Kitchen worked example live-proven on the rig, operator+publisher docs. `pack update` deferred to the v1.0 web presence. See docs/FREIGHT.md. | ✅ done (53f5e3a) |
| 3 | **Vitrine** | The operator walkthrough: all 15 Studio screens + TUI + desktop, friction notes → the polish backlog; inventory which `/classic` panes lack Studio equivalents. | ✅ done — walked live 2026-07-05 to 07-07, all 14 sections (0–13). See `docs/VITRINE.md` for the full, dated, severity-tagged finding list — it is the real backlog row 4 decomposes. |
| 4 | Polish waves | Fix the Vitrine backlog, batched by screen family. | ✅ done 2026-09-03 — see [`docs/POLISH_WAVES.md`](POLISH_WAVES.md). All 8 sequenced sub-projects shipped (small backlog sweep → `/classic` retirement → Repertoire completions → agent turn-quality fixes → Missions polish → UI modernization → config-write surface area → tool/server call-stat observability, the last split out of sub-project 2's own scoping). Nonagon role-based team templates was scoped out of v0.9 entirely (new product feature, not polish) — logged there as a v1.0-or-later candidate. |
| 5 | **Fleet panel** | Live specialist/mission feed: new streaming IPC over the Spyglass journal traces + the Studio screen (F3's deferred half). | ✅ done — shipped under the name **Chapter Mission Control** instead (2026-08-22/23; live LEAD/specialist graph, click-to-drill-in, gate approve/reject, abort, pause/resume). Currently in `CHANGELOG.md`'s `[Unreleased]`, not yet cut into a numbered release. |
| 6 | Governed-write completions | Studio "Add skill" (Tutor TU.3) + Repertoire approve-in-place / invocation history. | Folded into row 4's decomposition as sub-project 3 — see `docs/POLISH_WAVES.md`. |
| 7 | `/classic` retirement | Port the panes with no Studio equivalent (expect: sessions, notifications, full-audit browsing — read-only screen recipe), keep a minimal no-bundle fallback page, delete the rest. NOT a blind delete: `/classic` is also the no-bundle fallback today. | Folded into row 4's decomposition as sub-project 2 — see `docs/POLISH_WAVES.md` (real inventory is 4 panes, not 3; one already shipped). |
| 8 | Schedule-write autonomy gating | `schedule.create/update/delete` join the floor gated on the `[autonomy]` dial (the parked half of the 2026-07-04 schedule.list fix). | Folded into row 4's decomposition as part of sub-project 7 (the Schedules screen) — see `docs/POLISH_WAVES.md`. **Don't confuse with the Recursive-Scheduling Guard** (shipped 2026-08-25/26, `aivyx-ecosystem/ROADMAP.md`): that's a different, already-shipped, origin-based security gate, not this operator-facing autonomy-dial one. |

## Out of scope for v0.9

Verticals and web presence (v1.0 pillars), Nexus/Passport transport,
mobile (v1.0+ product decision: Tauri shell first, Flutter companion
second), any UI-stack migration.

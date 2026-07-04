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

| # | Chapter | Scope | Status |
|---|---|---|---|
| 1 | **Gatehouse** | Studio remote auth: `web_ui_token` bearer gate at one choke point (HTTP + WS upgrade) + the refuse-to-bind interlock (`insecure_no_auth` escape hatch) + install-time token generation + the Studio token prompt. Locked design in the v1.0 runway. Localhost-only installs untouched. | next |
| 2 | **Freight** | Paid-pack bundles: signed archive (binaries + config TOMLs + manifest w/ min-daemon-version), `aivyx pack install` (verify + Mise-pattern wiring), `aivyx pack update`. Locked design in the v1.0 runway. Kitchen = the free worked example. | queued |
| 3 | **Vitrine** | The operator walkthrough: all 15 Studio screens + TUI + desktop, friction notes → the polish backlog; inventory which `/classic` panes lack Studio equivalents. | queued (operator session) |
| 4 | Polish waves | Fix the Vitrine backlog, batched by screen family. | sized by 3 |
| 5 | **Fleet panel** | Live specialist/mission feed: new streaming IPC over the Spyglass journal traces + the Studio screen (F3's deferred half). | queued |
| 6 | Governed-write completions | Studio "Add skill" (Tutor TU.3) + Repertoire approve-in-place / invocation history. | queued |
| 7 | `/classic` retirement | Port the panes with no Studio equivalent (expect: sessions, notifications, full-audit browsing — read-only screen recipe), keep a minimal no-bundle fallback page, delete the rest. NOT a blind delete: `/classic` is also the no-bundle fallback today. | queued |
| 8 | Schedule-write autonomy gating | `schedule.create/update/delete` join the floor gated on the `[autonomy]` dial (the parked half of the 2026-07-04 schedule.list fix). | queued |

## Out of scope for v0.9

Verticals and web presence (v1.0 pillars), Nexus/Passport transport,
mobile (v1.0+ product decision: Tauri shell first, Flutter companion
second), any UI-stack migration.

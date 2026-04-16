# Phase 21 — Mission Primitive (P2)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Deliver the first concrete piece of **PRODUCT.md P2 — Mission
Primitive**: a long-running work item that survives across
process restarts, runs under a specific role's capability
envelope, and emits operator-visible approval gates as
`StreamEvent`s the channel adapter renders distinctively.

Phase 21 is a **product-shape phase** — it ships a new product
primitive and advances Product Commitment P2. It is the first
product-shape keystone since Phase 14 (Sub-Agent Role-Switching,
P1).

## Why now

1. **Both prerequisites are complete.** The daemon holds live
   state (P4, Phases 16–19) so missions can survive restarts.
   The role-config migration declares capability envelopes (P9,
   Phase 13) so missions can run under a specific role's scope.

2. **P2 is the next product-shape commitment on the critical
   path.** The product roadmap sequences P2 after P4 (daemon)
   and P9 (role-config). Both are delivered. The Mission
   Primitive milestone in `PRODUCT_ROADMAP.md` has been waiting
   for exactly this point.

3. **The deferral backlog is manageable.** Phase 20 reduced the
   rolling backlog from sixteen to ten items, none of which
   block mission work. A clean backlog means the phase can focus
   on product shape without deferral pressure.

4. **Missions are the load-bearing primitive for G5 (autonomous
   and scheduled execution).** The product contract's Goal 5
   explicitly depends on P2's mission semantics. Shipping the
   mission primitive now unblocks the goal-level commitment.

## Streak predictions

- **DESIGN.md** — Medium risk. The mission primitive may require
  a new design decision (mission storage schema, approval-gate
  protocol). Prediction: streak **may break** at twenty-one
  phases if an amendment is needed, or **extends to twenty-one**
  if the mission shape fits within existing design decisions.
- **PRODUCT.md** — Not at risk. Phase 21 advances P2, which is
  already committed. Prediction: streak extends to **nine
  consecutive phases**.
- **Production-core `aivyx-core/src/lib.rs`** — Medium risk.
  The mission primitive may need new infrastructure tool types
  or `ToolContext` extensions. Prediction: streak **may break**
  at ten phases, ending the record run.

## Tasks

### Task 1 — Phase open (this commit)

Scaffold `docs/PHASE_21.md`. Update `docs/README.md`
phase-status table (Phase 21 → Open). Update
`docs/ROADMAP.md` Phase 21 entry.

### Tasks 2+ — TBD

Mission primitive design and implementation tasks to be
scoped after the phase-open commit, based on investigation
of the approval-gate shape, mission storage model, and
daemon integration surface.

## Decisions

(None yet.)

## Open questions

**Q1 — What is a mission, concretely?** The product contract
deliberately does not pin the shape: "a row in redb, a long-
lived turn, a tree of sub-sessions" are all options. This
question must be resolved before Task 2 can be scoped.

(a) A mission is a redb row with a state machine
(created → running → gate-pending → completed/failed).
The daemon drives the state machine; each gate is a
`StreamEvent` the frontend renders.

(b) A mission is a long-lived `DaemonSession` with special
lifecycle semantics — it persists across frontend
disconnects and reconnects.

(c) A mission is a tree of sub-sessions (like P1's
`role.switch` but multi-turn and persistent).

**Recommendation: TBD — investigate in Task 2.**

**Q2 — What is the shape of an approval gate?** The product
contract says gates are "operator-controlled, not agent-
controlled" and that the agent "identifies decision points
where it judges operator approval is warranted." The gate
UX must work across both Local CLI and Telegram frontends.

**Recommendation: TBD — investigate in Task 2.**

**Q3 — Does the mission primitive need new capability scopes?**
PRODUCT.md mentions `mission.create` as a possible
infrastructure-tool capability base. If so, it needs to be
added to `KNOWN_BASES` and the tier ceilings.

**Recommendation: TBD — investigate in Task 2.**

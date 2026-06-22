# Close the kitchen loop — the PO-draft tool + a runnable overnight close (Chapter Lockup)

> **Status:** 🟡 **PLANNED (LK.0–LK.2).** A short follow-on to [Chapter
> Brigade](BRIGADE.md). Brigade gave the BOH brigade real `kitchen.*` tools, but
> left one gap: the flagship **`overnight_close_mission()`** delegates a
> `draft_po` step to purchasing — *"Draft per-supplier purchase orders for the
> low-stock items. Do not send them."* — yet purchasing only holds
> `kitchen.supplier.list` + the confirm-first `kitchen.order.send`. There is **no
> PO-draft tool**, so the mission's third step cannot be fulfilled. Lockup adds
> **`kitchen.order.draft`** (gated on the existing `kitchen.write` base — drafting
> is reversible; only *sending* spends money), wires it to purchasing, and proves
> the overnight close is end-to-end runnable: count → low-stock → **draft PO** →
> (human-gated) send, alongside the independent HACCP fridge round. **No new
> capability base, no P10 amendment, no core/daemon/team-engine change.**

## 1. Why this chapter

Brigade's verification stopped at "a specialist can invoke a `kitchen.*` tool."
The *vertical's reason to exist* is the **overnight close** — the nightly BOH
routine the pack's `overnight_close_mission()` encodes. That mission has four
steps; three now work (stocktake counts via `kitchen.inventory.*`, inventory
reads low-stock, HACCP logs the fridge round), but **`draft_po` is a dead step**:
its instruction is to *draft* POs, and purchasing has no tool that drafts. The
confirm-first `kitchen.order.send` only dispatches an *already-drafted* PO — so
today the agent would have to draft in KitchenDB/Flutter out-of-band before the
loop could even reach "send." Lockup closes that: purchasing can turn the
low-stock list into per-supplier draft POs, the mission runs as written, and the
only human gate is the deliberate one — sending.

## 2. Architecture & governance decisions (locked)

### `kitchen.order.draft` rides `kitchen.write` — not a new base, not confirm-first
Drafting a PO **writes a draft record** to KitchenDB; it is **reversible and
spends nothing**. So it is gated on the existing **`kitchen.write`** base (the
general kitchen-mutation scope, already in `KNOWN_BASES`) — **not** a new
`kitchen.order.draft` base (rejected: it would be a `KNOWN_BASES` change for no
safety gain), and **not** `kitchen.order.send` (rejected: that base is the
*outbound* power and stays narrowly the confirm-first send). The confirm-first
boundary therefore stays exactly where Brigade put it: **only
`kitchen.order.send` escalates.** `kitchen.order.draft` is a normal write
(`Verification::Unverified`, like the other BG.2 writes).

### Purchasing gains `kitchen.write` (scope) + `kitchen.order.draft` (tool)
The BOH pack's purchasing specialist currently holds `kitchen.read` +
`kitchen.order.send`. Lockup adds the `kitchen.write` **scope** (so the draft
tool passes the capability check) and the `kitchen.order.draft` **tool** to its
allowlist. Least privilege is preserved by the **allowlist**: purchasing can call
*only* `kitchen.supplier.list`, `kitchen.order.draft`, `kitchen.order.send` — it
holds `kitchen.write` but cannot adjust stock or run batches because those tools
aren't in its allowlist (the same allowlist-narrows-scope pattern stocktake
already uses). Updated in both the `kitchen_boh_team()` constructor and the
round-tripped `kitchen-boh.toml`; the BG.4 coherence test keeps them honest.

### "Runnable" is proven structurally + by a runbook — not a live LLM in CI
A full overnight-close run needs a live LLM driving the lead's
decompose/delegate. That is an **operator runbook**, not a CI dependency (same
stance as Brigade's live-KitchenDB §7). In-tree, "runnable" is proven
**structurally**: every `overnight_close_mission()` delegate step targets a real
specialist **whose allowlist now covers a `kitchen.*` tool that fulfils the
step** (in particular `draft_po` → purchasing → `kitchen.order.draft`), and the
toolkit's harness e2e already proves a `kitchen.*` invocation round-trips. A
deadstep guard test fails if a future edit reintroduces a step no specialist can
perform.

## 3. Scope

**In:** the `kitchen.order.draft` tool in `aivyx-kitchen-toolkit` (`kitchen.write`,
not confirm-first) + `all_tools()` registration (LK.1); wiring purchasing in the
`kitchen_boh_team()` constructor + `kitchen-boh.toml` + extending the coherence
test; a mission-satisfiability ("no dead step") test; the live overnight-close
runbook; finalize — full suite + clippy + `cargo deny`, chapter memory, COMPLETE
(LK.2). **Out:** a new capability base or P10 amendment; auto-*sending* POs (send
stays confirm-first and out of the mission); changing the mission's shape;
re-implementing PO-grouping logic (KitchenDB's `draft_purchase_order` RPC owns
per-supplier grouping / pack-size rules); richer HACCP audit fields (still
deferred from Brigade BG.4); a second vertical.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **LK.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **LK.1** ✅ | **`kitchen.order.draft` tool** | DONE. New tool in `order.rs`: optional `items` (array of `{sku, quantity}`, each validated to non-empty sku + positive qty) + optional `notes` → `draft_purchase_order` (no items → empty params, KitchenDB auto-drafts from low-stock; client injects `p_organization_id`). **`kitchen.write` scope** (not confirm-first; the confirm-first boundary stays on `order.send`), `run_write`. Added to `all_tools()` (→ **11 tools**); the harness e2e + `all_tools` registry tests updated to 11. +5 tests (empty/items+notes mapping, bad-items rejects, name/scope-is-write). Crate at 45 tests + coherence + e2e; clippy `-D warnings` green. |
| **LK.2** | **Wire purchasing + prove the loop + finalize** | Add `kitchen.write` scope + `kitchen.order.draft` to purchasing in `kitchen_boh_team()` + `kitchen-boh.toml`; extend `boh_coherence.rs`; add a "no dead step" mission test (every `overnight_close_mission()` delegate step's specialist holds a fulfilling `kitchen.*` tool). Live overnight-close runbook. Full workspace suite + clippy `-D warnings` + `cargo deny`; chapter memory; status → COMPLETE. |

**Discipline:** `kitchen.order.draft` reuses the BG.1 client + BG.2 `run_write` —
no new client mechanic. The purchasing wiring touches the constructor **and** the
round-tripped TOML together (the BG.4 round-trip test guards drift). Test band:
**small** — one tool (param-map + name/scope) + the wiring/coherence/dead-step
tests; price **~8–14 new tests**.

## 5. Open questions (resolve in-phase)

- **OQ-1 — draft input shape (LK.1).** Optional `items` + `notes`, else KitchenDB
  auto-drafts from current low-stock (lean — lets the agent pass an explicit
  reorder list *or* defer to the DB) vs. a required explicit `items`. The exact
  `draft_purchase_order` params are confirmed against the live schema in-phase
  (Brigade OQ-3 carries forward).
- **OQ-2 — draft scope (LK.1).** `kitchen.write` (locked — reversible mutation,
  no new base, confirm-first boundary stays on `order.send`) vs. a dedicated
  `kitchen.order.draft` base (rejected — a `KNOWN_BASES` change for no safety
  gain). Revisit only if an operator wants to grant *drafting* without any other
  kitchen write, which the allowlist already separates at the tool level.

---

*Chapter Lockup is the last turn of the key. Brigade handed the brigade its
knives; Lockup lets purchasing actually write the order it was told to draft — so
the overnight close runs end to end as the pack always described it, count to
fridge round, with the single human gate exactly where it belongs: the moment the
order is sent.*

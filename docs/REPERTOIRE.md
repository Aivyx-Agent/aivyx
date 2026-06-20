# The Skills Library — a Studio screen (Chapter Repertoire)

> **Status:** 📚 **RP.2 — the Studio Skills screen shipped.** A `View::Skills`
> + nav entry + `SkillsPanel`: one card per skill (name, provenance badge
> operator/agent, `domain` chip, version, "refined from …" lineage, trigger, an
> effectiveness bar + bucket label off the WH.2 EWMA, and the procedure body in
> a collapsible `<details>`), effectiveness-descending with unmeasured last, plus
> a "N pending — review in Agents" banner that switches view. Bundle rebuilt +
> `dist/` committed. With RP.1's data path the skills library is live. Only
> finalize (RP.3) remains. The locked reference for the
> Studio's **Skills library**: the screen where the operator sees the
> agent's whole **repertoire** of skills — operator-taught, agent-authored
> ([[PRAXIS]]), and agent-refined ([[WHETSTONE]]) — each with its
> **effectiveness**, provenance, and lineage. Today skills are scattered:
> taught in chat, proposed in the Agents screen, invoked invisibly mid-
> turn, and *measured* (the WH.2 effectiveness ledger) with **no surface
> at all**. Repertoire gives them a home. A read-only Studio screen over
> new read-only IPC — the same shape as the Memory / Wiki / Graph screens;
> **no new capability base, tool, or amendment.**

## 1. Why — skills are everywhere and nowhere

After Whetstone + Praxis the agent has a real, growing set of skills, and
a per-skill effectiveness signal. But an operator can't *see* any of it as
a whole:

- **No inventory.** There's no one place that lists every skill with its
  full body — `skills.list` is an agent tool, the Agents screen shows the
  persona but not the skills as first-class objects.
- **The effectiveness ledger is invisible.** WH.2 measures how well each
  skill performs (a decayed EWMA + sample count); nothing renders it. The
  operator can't tell which skills are pulling their weight.
- **No provenance/lineage view.** Was this skill taught, authored from
  knowledge, or refined from an earlier version? The WH.1
  `provenance`/`refined_from`/`version`/`domain` fields exist but are
  never shown.

Repertoire surfaces all three — turning the skill machinery the last two
chapters built into something the operator can actually inspect and trust.

## 2. Architecture & decisions (locked)

### A read-only Studio screen over read-only IPC
Exactly the Memory / Wiki / Graph pattern: a new `GetSkills` query +
response on the wasm-clean `aivyx-ipc` protocol, a `handle_query` arm that
reads existing daemon state, and a new `View::Skills` panel in the Studio
(`aivyx-web`). No daemon *writes*, no new capability surface — the screen
only *shows*.

### `GetSkills` → the inventory + its effectiveness
A new IPC pair: `QueryPayload::GetSkills` →
`QueryResponsePayload::GetSkills { skills: Vec<SkillView>, pending_proposals: usize }`,
where **`SkillView`** is a wasm-clean type in `aivyx-ipc`:

```text
SkillView {
    skill: LearnedSkill,   // name, trigger, procedure, version,
                           //   provenance, refined_from, domain (WH.1)
    ewma_score: f32,       // decayed effectiveness (WH.2 ledger), 0 if unseen
    samples: u32,          // folded windows — a confidence proxy
}
```

The `handle_query` arm assembles it from state it already has: the
**effective persona's `learned_skills`** (decoded `LearnedSkill`s) joined
with the **WH.2 `SkillEffectivenessLedger`** (a `skill_score` lookup per
name; absent → 0/0, "not yet measured"). It also counts **pending
LearnedSkill-category proposals** in the proposal log, for the governance
pointer below. `handle_query` gains one new parameter — the effectiveness
ledger handle — threaded from the daemon like the wiki/graph stores.

### Governance stays in Agents — the screen *points*, doesn't duplicate
Skill **proposals** (Whetstone refinements, Praxis authored skills) are
persona proposals and already have a full approve / edit / reject surface
in the **Agents** screen. Repertoire does **not** re-implement that — it
shows a *"N pending skill proposals — review in Agents"* banner that links
across. The library's unique value is the **inventory + effectiveness +
lineage** (what Agents doesn't show); approval lives where it already
works. (Approve-in-place is a documented deferral.)

### The screen — cards, badges, effectiveness, the body on demand
A `SkillsPanel` rendering one card per skill: the **name** + a
**provenance badge** (`operator` / `agent`), a **`domain`** chip when set,
the **version** (and *"refined from …"* lineage when `refined_from` is
set), the **trigger** always visible, and an **effectiveness** indicator
(the EWMA as a small bar/score + the sample count, with an "unmeasured"
state). The full **procedure** body is collapsed by default (it can be
long — the same reason the prompt elides it) and expands on click. Sorted
sensibly (e.g., effectiveness, or provenance then name). Stitch-styled,
offline, reusing the existing component kit + the empty-state pattern
("No skills yet — teach one with `skills.teach`, or enable
`[skill_authoring]`").

### Governance: a read-only screen, nothing new
No new capability base, agent tool, P10 amendment, or storage domain. New
read-only IPC + a daemon read arm + a WASM screen — the established
Studio-screen recipe (Chapters S/T/Codex/Lattice). The bundle is rebuilt
and `dist/` committed, as every web chapter does.

## 3. Scope

**In:** the `GetSkills` IPC pair + the `SkillView` wasm-clean type; the
`handle_query` arm (persona `learned_skills` ⋈ effectiveness ledger +
pending-proposal count) + threading the ledger handle into `handle_query`;
the `View::Skills` Studio screen (nav + panel + cards + badges +
effectiveness + expandable body + the Agents pointer); the bundle rebuild
+ `dist/` commit.

**Out:** approve / edit / reject **in** the Skills screen (it points to
Agents — that governance already exists); editing or deleting a skill from
the screen (a future write-screen step); any new agent tool / capability
base; surfacing skill *invocation history* (a possible later enrichment).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **RP.0** | **This design contract** | locked reference; banner flips per phase |
| **RP.1** ✅ | **`GetSkills` IPC + daemon arm** | DONE. `SkillView { skill: LearnedSkill, ewma_score, samples }` in `aivyx-ipc::protocol` + `QueryPayload::GetSkills` / `QueryResponsePayload::GetSkills { skills, pending_proposals }` (roundtrip test). `handle_query` gained a `skill_effectiveness_ledger` param (threaded at the call site like wiki/graph); the `GetSkills` arm reads `shared_persona.read().learned_skills` (decoded), joins each with `ledger.skill_score(name, now)` (absent → `0/0`), and counts Pending `LearnedSkill`-category proposals via `proposal_log.list(Pending)`. ipc 61 / channel 1008 + clippy green. |
| **RP.2** ✅ | **The Studio Skills screen** | DONE. `View::Skills` + nav entry + `SkillsPanel`/`SkillCard` (name, provenance badge, `domain` chip, `v{n}`, "refined from …", trigger, a `skill_effectiveness` bucket label + bar off the WH.2 EWMA, `<details>` procedure), sorted effectiveness-desc (unmeasured last), + a `skills-pending` banner that `view.set(View::Agents)` (so `view` is now a context provider) + the `get_skills()` ws call + the `GetSkills` fan-in arm + `SkillsState`. Stitch CSS for the cards. WASM bundle rebuilt (`dx bundle --release`) + `dist/` committed (the new strings are in the wasm; existing screens intact). web clippy 0. |
| **RP.3** | **Finalize** | full suite + clippy + `cargo deny` green; README (Studio screen count Twelve→Thirteen) + CHANGELOG + FRONTEND screen row; status flip; record. |

**Discipline:** RP.1 is read-only IPC + a read arm (no behaviour change —
the data already exists). RP.2 is the WASM screen (the only bundle
rebuild). The screen *shows*, never mutates; governance stays in Agents.
The chapter adds *visibility*, not capability — the skill machinery the
operator already owns, finally legible.

## 5. Open questions (resolve in-phase)

- **Default sort** — effectiveness-descending (surface what works /
  what's failing) vs. provenance-then-name (stable, predictable). Default
  effectiveness-descending with the unmeasured grouped last (RP.2).
- **Effectiveness rendering** — a raw EWMA number vs. a normalized
  bar/label (e.g., "helping / neutral / underperforming" buckets off the
  same WH.3 floor). Default a small bar + the bucket label + sample count
  (legible without explaining EWMAs) (RP.2).
- **Reuse the existing Wiki/Graph empty-state + card components** vs. a
  bespoke skill card. Default reuse the kit; a skill card is just a
  titled card with badge chips (RP.2).

---

*Chapter Repertoire is where the agent's skills stop being invisible
plumbing and become a library the operator can read: every skill it was
taught, authored, or refined — what it does, where it came from, and how
well it actually works — on one Stitch-styled screen.*

# Phase 184 — Conversational Skill-Teaching

**Chapter H, phase 5** — the last named Chapter H phase. Today
the **skills** layer (the `LearnedSkill` entries in the Persona
chain that `skills.list` / `skills.invoke` expose) is
*author-facing*: skills appear only when the Phase 113
reflection auto-proposer *detects* a repeated procedure and the
operator *approves* it. The End User can't simply **teach** the
agent a skill. Phase 184 closes that — *"let me show you how I do
X: first…, then…"* → the agent captures it, confirms, and saves
it as a callable skill. With Phase 181 (identity) and the
existing roles, this completes the "**fully customizable**"
promise: Profile, Persona, Roles — and now skills — are all
user-shaped.

## What a skill is (the substrate we build on)

A skill is a `LearnedSkill { name, trigger, procedure }` stored
as a JSON list-entry under `PersonaDeltaCategory::LearnedSkill`
(`AppendList` to add, `RemoveList` to drop). The Persona chain is
HMAC-chained + append-only — it *is* the audit trail. So:

- **Teach** a new skill → `AppendList { LearnedSkill }`.
- **Update** one → `RemoveList { old }` + `AppendList { new }`.
- **Forget** one → `RemoveList { old }`.

## The approval model (operator-chosen)

A skill is a Persona (P14) delta, normally operator-gated. The
operator chose **draft-in-chat, save on confirmation**: the
teaching *is* the authorization. The agent drafts the skill from
the conversation, **shows it back** ("Here's the skill I'll
save: `<name>` — *when:* `<trigger>` — *steps:* `<procedure>` —
confirm?"), and only on the operator's explicit "yes" saves it
**directly** to the chain. No CLI detour; the confirmation guards
against mis-capture.

### Making "confirm first" safe-by-contract

Because a tool can't *see* whether the agent really confirmed,
the confirmation is an **explicit, audited part of the tool
contract**: `skills.teach` / `skills.update` / `skills.forget`
require a `confirmed: true` field, and the tool **rejects**
`confirmed` absent/false with "draft the skill, show it to the
operator, and only set `confirmed: true` after they approve."
Layered with: **Trusted-tier only** (a SemiTrusted remote
adapter can't teach skills — they modify identity), the full
**HMAC persona-chain audit**, and **revertibility** (`aivyx
persona revert`). The agent saving without confirming is a
visible, reversible event, not a silent identity change.

## Design

- **`skills.write` base** — one new capability base gating all
  three edit tools (Trusted-tier, like the existing
  `skills.propose` / `skills.list` / `skills.invoke`). Via the A3
  capability-growth process (`KNOWN_BASES` + tripwire +
  amendment), **not** a `DESIGN.md` edit.
- **Pure delta builders** — `teach_delta(skill)`,
  `forget_delta(existing)`, `update_deltas(old, new)` translate a
  `LearnedSkill` change into the Persona delta op(s); plus
  `find_skill_by_name(skills, name)` and name validation
  (kebab-case, non-empty, not a duplicate on teach). Unit-tested
  without a chain.
- **The three channel-tier tools** — `skills.teach`,
  `skills.update`, `skills.forget`. Each reads the current
  `LearnedSkill` set (the injected effective-persona snapshot),
  validates, requires `confirmed: true`, builds the delta(s), and
  appends to the persona chain. Mirrors the `reflection_tool`
  persona-log injection pattern.
- **Confirm-first agent guidance** — a short system-prompt
  protocol so the agent *always* drafts + shows + confirms before
  calling an edit tool, and the tool descriptions repeat it.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 183's frozen hash (`b9e59d0`).
2. **`skills.write` base + pure delta builders.** The base
   (`KNOWN_BASES` + `CEILING_TRUSTED` + tripwire 74→75 + A3
   amendment) + `teach_delta` / `forget_delta` / `update_deltas`
   / `find_skill_by_name` / name validation. Tests for each
   (add / remove / update-as-remove+add / dup-name rejection /
   not-found).
3. **`skills.{teach,update,forget}` tools.** Channel-tier tools
   reading the current skill set + appending deltas to the chain;
   the `confirmed: true` gate; persona-chain + snapshot
   injection; bin registration. Tests: each tool's
   save/update/forget path, the `confirmed`-required rejection,
   not-found handling.
4. **Confirm-first guidance + INSTALL.** The system-prompt
   protocol section + tool-description wording; INSTALL section
   (teach / update / forget, the confirmation contract, the
   Trusted-tier + audit + revert posture). Tests on the guidance
   presence.
5. **Exit + Frozen.** Exit doc; README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 20 → **21**. New
  channel-tier tools + one capability base — the same additive
  shape as the Phase 172/173/183 channel-tier tools, none of
  which touched the contract. The 13-tool substrate cap is
  untouched; skills write the existing Persona chain.
- **PRODUCT.md** — **Will hold.** Streak: 74 → **75**. A new way
  to populate the *existing* P14 `LearnedSkill` layer (operator
  authorship alongside reflection authorship); not a new
  commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 20 →
  **21**. The edit tools are channel-tier (they write the persona
  chain, which lives in `aivyx-channel`); `aivyx-core` —
  including the existing substrate `skills.list` / `skills.invoke`
  — is untouched.

## Exit criteria

- [ ] `docs/PHASE_184.md` + README row + Phase 183 backfill — T1.
- [ ] `skills.write` base + pure delta builders — T2.
- [ ] `skills.teach/update/forget` with the `confirmed` gate,
  writing the persona chain — T3.
- [ ] Confirm-first system-prompt guidance + INSTALL — T4.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+20`. *(Priced by components:
  pure delta builders + three thin chain-writing tools + a
  guidance assertion. No parser, no IPC surface, no new
  substrate — NOT a coarse "substrate" band. The Phase 183
  over-pricing lesson applied.)*

## Honest scope risks at sign-off

- **The `confirmed` gate is a protocol, not a hard lock.** The
  tool requires `confirmed: true`, but the agent supplies it —
  the safeguards are Trusted-tier + audit + revert, not a second
  human approval. (The operator who wants the strict gate keeps
  using the reflection-proposed-then-`approve` path.)
- **Find-by-name is exact.** `skills.update` / `skills.forget`
  match a skill name exactly; a near-miss returns "no such
  skill" with the available names listed, rather than guessing.
- **Capture quality is the agent's.** A weak model may draft a
  thin `procedure`; the in-chat confirmation + `skills.update`
  are the corrections.
- **No skill versioning.** `update` is remove-then-add; the prior
  version is recoverable from the append-only chain but not
  surfaced as "history."

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 21 | Untouched (channel-tier tools + one `skills.write` base via A3; 13-tool cap untouched) | ✅ |
| PRODUCT.md HOLD → 75 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 21 | Untouched (edit tools are channel-tier; core `skills.list/invoke` untouched) | ✅ |
| Zero new workspace deps | reused the persona chain + capability + sha256 | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+12` to `+20` | **`+11`** (skill_edit 6 + skill_tool 4 + prompt 1); ~4,195 → ~4,206 | ⚠️ **one below band** |

**Band note — a near-hit, low by one.** Component-pricing put
this at `+12..+20` (delta builders ~6, three tools ~8, guidance
~2). It landed `+11`: the builders + guidance were on target, but
I wrote the tool tests **denser** than estimated — one
round-trip covering teach→update→forget, one unconfirmed-reject,
one duplicate/missing-reject, one scope = 4, not ~8. So the
estimate's *shape* was right (no coarse-label regression like
Phase 183); I just under-counted by consolidating. A clean miss
of one, recorded honestly.

What shipped, end-to-end:

1. **`skills.write` base + pure builders** (T2). `teach_op` /
   `forget_op` / `update_ops` / `merged_skill` /
   `find_skill_by_name` / `validate_skill_name`, unit-tested
   without a chain.
2. **The three tools** (T3). `skills.teach` / `update` / `forget`
   read the current skill set, require `confirmed: true`, append
   `LearnedSkill` deltas to the persona chain, recompute the
   effective persona.
3. **Confirm-first guidance** (T4). The tool contract enforces
   it always (the `confirmed` field); the skills prompt section
   reinforces it; INSTALL documents the flow.

### Honest scope risks at sign-off

- **The `confirmed` gate is a protocol, not a second human
  approval** — the safeguards are Trusted-tier + audit + revert.
- **Find-by-name is exact** — a near-miss lists the available
  names rather than guessing.
- **Capture quality is the agent's** — the in-chat confirmation
  + `skills.update` are the corrections.

### The result

Chapter H is **complete**. The "fully customizable" promise is
realized end-to-end: Profile (Phase 181 guided builder), Persona
(reflection-grown), Roles, and now **skills the End User teaches
in conversation** — all user-shaped, on the existing HMAC persona
chain, with the 13-tool substrate cap untouched and an unbroken
DESIGN / PRODUCT / `lib.rs` streak across the whole chapter.

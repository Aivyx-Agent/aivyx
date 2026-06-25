# Agent Autonomy — one dial the end user controls (Chapter Reins)

> **Status:** design contract. This is the spec Chapter Reins scaffolds from
> (mirrors `docs/ACCESS_LEVELS.md` / `docs/HEADLESS_MODE.md`).
>
> Today, *how autonomous the agent is* is the emergent product of five
> independent knobs — `[access] level`, `confirm_destructive`, the headless
> `gate_policy`, whether the `[loop]` is armed, and `[budget]` caps — plus the
> propose-only self-improvement loops. Each is individually sound, but no end
> user can answer the simple question *"how much rope does my agent have?"*
> without reasoning about all five at once.
>
> **Chapter Reins** introduces **one end-user-facing choice** — `[autonomy]
> level` — that composes those knobs into named, coherent tiers, exactly as
> `[access] level` composes `fs_root` + shell reach. The end user picks one
> word; power users still set the underlying knobs directly. The default
> reproduces today's behavior **byte-for-byte**. Autonomy becomes a *choice the
> operator makes*, never a property the agent grows into on its own.
>
> Companion to the runaway-bounding work (the [Bridle](BRIDLE.md) +
> Halter cycle breakers) and the unattended-gate policy
> ([HEADLESS_MODE.md](HEADLESS_MODE.md)): those bound a *single run*; Reins
> bounds *the standing posture*. Read [`SECURITY_POSTURE.md`](SECURITY_POSTURE.md)
> first — this chapter is the operator-facing front end to that containment
> model.

---

## 0. Decisions (locked at scope time)

| Decision | Choice | Why |
|---|---|---|
| Default level | **`assisted`** = today's behavior | Absent `[autonomy]` ⇒ byte-identical to the current shipped posture (reversible-free, irreversible-confirmed, no loop, propose-only growth). Expansion *and* restriction are both opt-in. |
| The dial is a **composition**, not new authority | A level **expands** into the existing knobs (`[access]`, `confirm_destructive`, `gate_policy`, `[loop]`, `[budget]`, growth policy) | No new capability base, no new authority surface. Reins is a *front end* to primitives that already exist and are already enforced. |
| Explicit knobs win | A directly-set knob **overrides** the level's expansion | Same override semantics as `[memory] profile` and `[access] level`. Power users keep full control; the dial is a convenience, never a cage. |
| Granularity | **Global level + per-domain overrides** | "Autonomous at coding, manual on money/email" is the real-world ask. A flat dial can't express it; a global posture + keyed exceptions can. |
| Identity is never self-adoptable | Persona, access level, trust ceilings, **and the autonomy level itself** are always human-gated — at *every* tier | The agent may grow its skills and pursue goals autonomously; it may never rewrite *who it is* or *how much rope it has*. This is the line that makes every tier safe. |
| The safe ceiling | Even `autonomous` **never** auto-approves irreversible/outbound-money actions | Reject-and-abort stays the unattended posture for irreversible ops (Chapter H invariant). Higher tiers get *more reach* and *bounded auto-approval of reversible work*, not a money/delete rubber stamp. |
| The expert escape hatch | A **separate, loudly-named, acknowledgment-gated** config can lift the irreversible block — for an isolated, dedicated host only | Operators running the agent on a throwaway VM as a genuine infra-operator have a legitimate need. It is **off the safe path by construction**: a distinct flag, not a tier; default off; refuses without an explicit typed acknowledgment; every use is a first-class audit event. See §6. |

---

## 1. What exists today (the knobs Reins composes)

| Knob | Lives in | Controls |
|---|---|---|
| `[access] level` | `aivyx-config` (Chapter N) | filesystem + shell reach (`fs_root`) |
| `confirm_destructive` | `[access]` (Chapter N) | whether irreversible ops escalate to a confirm-first gate |
| `gate_policy` | `aivyx-core::GatePolicy` (Chapter H) | what an *unattended* run does at a gate: `Interactive` (park) vs `RejectAndAbort` |
| `[loop]` arm + caps | `aivyx-config` (Phases 173–176, K) | the autonomous loop: `max_iterations` / `max_run_secs` / `max_run_tokens` / `max_run_usd` + the post-iteration verification gate |
| `[budget]` | `aivyx-cost` (Chapter K) | per-turn dollar/token spend caps that alert or deny |
| growth governance | persona-proposal loop (Chapters V/W/X/Praxis/Whetstone) | self-improvement is **propose-only**: skills + persona changes surface for human approve/edit/reject |

All six are real and enforced. None is *named* from the end user's point of
view, and none knows about the others. Reins ties them together.

---

## 2. The autonomy levels

| Level | Gate posture | Loop | Self-improvement | Goal-setting | For |
|---|---|---|---|---|---|
| **`manual`** | confirm *everything* (even reversible) | off | off | none | a pure assistant; maximum oversight |
| **`assisted`** *(default)* | reversible free; irreversible → confirm | off | propose-only (governed) | none | the everyday personal assistant |
| **`supervised`** | irreversible → *batched* approval (queue, review together) | armed, capped, **per-iteration gate** | auto-adopt *low-risk* skill refinements; persona still gated | agent **proposes** a backlog | a builder with a human nearby |
| **`autonomous`** | irreversible → **reject-abort** unattended; reversible auto-approved within the allowlist | armed, capped, unattended | auto-adopt within effectiveness policy; persona still gated | agent **proposes and pursues** within budget | eyes-on-dashboard operation |
| **`unleashed`** | `confirm_destructive` off (explicit warning); reject-abort still holds for money/outbound unless §6 hatch is set | armed, operator-capped | broad auto-adopt; persona still gated | self-directed within budget | dedicated isolated host, eyes-open |

Each row is a **named expansion** of §1's knobs — see §4. Two properties hold
across *every* row, `unleashed` included:

- **Identity stays human-gated.** Persona, access, trust ceilings, and the
  autonomy level are never self-adoptable (Decision: "Identity is never
  self-adoptable").
- **The agent can't move its own dial.** Raising `[autonomy] level` is
  `config.write` — Kernel-tier, unreachable from an agent turn (§5 of
  `SECURITY_POSTURE.md`).

---

## 3. The `[autonomy]` config section

```toml
[autonomy]
# One word the end user owns. Default "assisted" == today's behavior.
level = "supervised"

# Optional: scope-keyed exceptions to the global level. `domain` is a
# capability-domain label (the tool name's first segment / group base):
# "shell", "fs", "git", "email" (gmail.*/calendar.*), "kitchen", "web", …
[[autonomy.override]]
domain = "email"
level  = "manual"          # always ask, regardless of the global tier

[[autonomy.override]]
domain = "shell"
level  = "autonomous"      # trusted to run commands unattended

# Optional: the reversible-action allowlist that bounded AutoApprove consults
# at `autonomous`+ (§5.1). Scopes NOT listed here still park/reject; this only
# ever widens *reversible* auto-approval, never confirm-first/irreversible.
[autonomy.auto_approve]
scopes = ["fs.write", "net.fetch", "git.read", "data.csv"]
```

**Resolution order** for any given tool call: the most specific
`[[autonomy.override]]` whose `domain` matches the call's scope wins;
otherwise the global `level`; explicit low-level knobs (`[access]`,
`gate_policy`, `[loop]`, `confirm_destructive`) **override** whatever the
level would have set. The effective posture is computed once at config load
and is itself an audited value (`ConfigChanged` shows the expansion).

---

## 4. Composition semantics (how a level expands)

A level is a pure function `AutonomyLevel -> AutonomyPosture`, where
`AutonomyPosture` is the bundle of low-level knobs. Sketch:

| Level | `gate_policy` | loop | `confirm_destructive` | growth adoption |
|---|---|---|---|---|
| `manual` | Interactive, confirm-all | disabled | on (+reversible) | none |
| `assisted` | Interactive | disabled | on | propose-only |
| `supervised` | Interactive, **batched** | armed (caps required) | on | low-risk auto |
| `autonomous` | **RejectAndAbort** + AutoApprove(allowlist) | armed (caps required) | on | policy auto |
| `unleashed` | RejectAndAbort + AutoApprove | armed (operator caps) | **off** | broad auto |

Rules:

1. **Default identity.** No `[autonomy]` section ⇒ `assisted` ⇒ the exact knob
   values the daemon uses today. The expansion of `assisted` is asserted
   byte-identical to current behavior by a test (the "no behavior change when
   off" guarantee, like every prior composition chapter).
2. **Explicit wins.** If the operator set `gate_policy` / `[loop]` / `[access]`
   / `confirm_destructive` directly, the level does **not** clobber it. The
   level only fills *unset* knobs.
3. **Caps are mandatory above `assisted`.** `supervised`+ requires at least one
   loop cap (`max_iterations` always has a default; the dollar cap is strongly
   recommended and warned-if-absent). A tier can grant the loop; it can't grant
   an *uncapped* loop except `unleashed`, which warns loudly.
4. **Remote channels are unaffected.** The tier governs the *Local/Trusted*
   operator surface. Remote channels keep their tier-ceiling attenuation (no
   shell, narrow fs) at every autonomy level — Reins never lets a Telegram
   message inherit `autonomous`.

---

## 5. The three ceiling-raising safety primitives

These let the higher tiers *accomplish more* without *risking more*. Each is
an independent opt-in and each is built so the dangerous door stays shut.

### 5.1 Bounded `AutoApprove` (the deferred Chapter H increment)

Today unattended = reject-abort: safe, but it also blocks *reversible* work
that merely happened to escalate. `AutoApprove` adds a **third** `GatePolicy`
posture, consulted only at `autonomous`+:

- Auto-approve an escalation **iff** its required scope is on the
  `[autonomy.auto_approve] scopes` allowlist **and** the tool is not
  confirm-first/irreversible.
- **Never** auto-approve a confirm-first/irreversible/outbound-money tool
  (`fs.delete`, destructive shell, `kitchen.order.send`, the `confirmed:true`
  pattern). This is a structural exclusion in the policy type, not a list entry
  — the allowlist *cannot* express it.

So `autonomous` can fetch, write within `fs_root`, read git, parse data
unattended — and still *cannot* delete, overwrite-destructively, or send money
without a human. (The §6 hatch is the only thing that lifts the irreversible
exclusion, and only on an isolated host.)

### 5.2 Checkpoint / rollback (reversibility as a lever)

The cheapest way to raise the autonomy ceiling is to make more actions
*reversible*. A `checkpoint.*` capability (git-backed workspace snapshot, fs
snapshot, or an op-log) lets the agent mark a restore point before a batch and
roll back. An action that is *cleanly reversible* can be auto-approved at
`autonomous` where its irreversible cousin cannot. Gated per tier; the
checkpoint store is itself on the audit chain.

### 5.3 Capability *request* channel (safe escalation)

The agent still cannot self-escalate (Kernel-gated — kept). But when it hits a
wall it can **request** a capability with a written justification → a governed
proposal in the same approve/edit/reject UI as persona proposals. The operator
grants it once (audited as a deliberate `CapabilityGranted` event). This turns
"the agent is stuck" into a reviewable ask instead of a silent dead end —
widening reach *with* a human in the loop, never around one.

---

## 6. The expert escape hatch (the one deliberate exception)

Operators who run the agent as a genuine infrastructure operator on a
**dedicated, isolated, disposable host** have a legitimate need to let it take
irreversible actions unattended (provision/destroy cloud resources, rewrite
system state). Reins serves this need **off the safe path, by construction**:

```toml
[autonomy]
level = "unleashed"

# DANGER. Lifts the structural block on auto-approving IRREVERSIBLE and
# outbound-money actions in unattended runs. Intended ONLY for a dedicated,
# isolated, disposable host where the agent's blast radius IS the host.
# Default false. Refuses to take effect without the typed acknowledgment.
unsafe_allow_irreversible_unattended = true
acknowledge = "I accept this host is the agent's blast radius"
```

Fences (all enforced, not advisory):

1. **Not a tier.** It is a *separate flag*, never reachable by picking a level
   word. You must name it, and you must be at `unleashed` for it to even parse.
2. **Default false.** Absent ⇒ the §5.1 irreversible exclusion holds, even at
   `unleashed`.
3. **Typed acknowledgment required.** The flag refuses to take effect unless
   `acknowledge` matches the exact required sentence. No accidental enablement.
4. **Loud and audited.** Enabling it emits a distinct `UnsafeAutonomyEnabled`
   audit event at boot, prints a red banner on every daemon start, and is
   surfaced in `aivyx doctor` and the Studio with a warning treatment.
5. **Still capped.** Budget/iteration/wall-clock caps and the HMAC audit chain
   still apply. The hatch lifts the *irreversibility* block; it does not lift
   *accountability* or *spend* limits. There is no flag that removes the audit
   chain — that invariant has no escape hatch.
6. **Still no self-escalation.** Even here, identity/access/the autonomy level
   stay human-gated. The agent can reshape the *host*; it cannot rewrite
   *itself* or grant *itself* this flag.

The documentation stance is explicit: this is **not recommended**, it is
**supported for the isolated-host use case**, and choosing it is opting into an
agent that can reshape the machine on its own. `SECURITY_POSTURE.md` §7–8 is
the required reading it links to.

---

## 7. Self-improvement graduation (growth under the dial)

Today *all* self-improvement is propose-only — the safe floor. The dial lets
the end user graduate it without ever touching the identity line:

- **Effectiveness-gated skill adoption.** Whetstone/Praxis already score skills
  (decayed EWMA + sample count). At `supervised`+, a refinement above a
  configurable effectiveness threshold *with enough samples* auto-adopts;
  below it, it stays a proposal. The bar, not the human, becomes the gate.
- **Self-authored goals.** Reflection already runs on a cadence; let it emit
  *backlog items* with rationale. At `supervised` they queue for approval; at
  `autonomous` the agent pursues them within budget. The agent begins directing
  its own improvement, bounded by caps + the verification gate + the audit
  chain.
- **Learned auto-approval (the flywheel).** The agent notices a class of action
  it has been approved for N times and *proposes promoting* that scope onto the
  `[autonomy.auto_approve]` allowlist. The human confirms the promotion once;
  thereafter that reversible class runs friction-free. Repetitive gates become
  *learned policy* — growth and autonomy compounding — with a human-confirmed
  promotion step keeping it honest.

**The fixed line, restated:** persona/identity, access, trust ceilings, and the
autonomy level are never self-adoptable at any tier. Skills and goals grow
autonomously; *who the agent is* and *how much rope it has* change only by a
human hand.

---

## 8. Invariants (the lines that do not move)

- **Default is `assisted` = today.** No `[autonomy]` section ⇒ byte-identical
  behavior; a test asserts the expansion.
- **The dial is composition, not authority.** No new capability base; a level
  only ever sets knobs that already exist and are already enforced.
- **The agent can't move its own dial.** Autonomy level is `config.write`
  (Kernel-tier). Self-escalation remains impossible.
- **Identity stays human-gated at every tier.** Persona/access/trust/level
  changes are never self-adoptable, `unleashed` and hatch included.
- **Irreversible/money is reject-abort unattended** — except behind the §6
  acknowledgment-gated hatch on an isolated host, which still keeps caps + audit.
- **The audit chain has no escape hatch.** Every action, refusal, auto-approval,
  and unsafe-mode boot is on the one HMAC chain. Always.
- **Remote channels never inherit a tier.** Tier-ceiling attenuation holds at
  every autonomy level.

---

## 9. Phase plan (Chapter Reins)

| Phase | Deliverable |
|---|---|
| **RN.0** | This design contract (`docs/AUTONOMY.md`), committed. |
| **RN.1** | `AutonomyLevel` + `AutonomyPosture` types (aivyx-config/core); pure `level → posture` expansion; `assisted`-is-today byte-identical test. No wiring yet. |
| **RN.2** | `[autonomy]` config section + per-domain overrides + resolution order; expansion fills *unset* knobs only (explicit-wins); `ConfigChanged` shows the expansion. |
| **RN.3** | Bounded `AutoApprove` `GatePolicy` (§5.1) + the `[autonomy.auto_approve]` allowlist; irreversible exclusion is a type-level structural property. |
| **RN.4** | The expert escape hatch (§6): flag + typed acknowledgment + `UnsafeAutonomyEnabled` audit event + boot banner + `doctor`/Studio surfacing. |
| **RN.5** | Self-improvement graduation (§7): effectiveness-gated adoption + self-authored goals + learned-auto-approval proposal. Opt-in per tier. |
| **RN.6** | Surfaces: `aivyx autonomy [show|set <level>]` CLI + a Studio Settings "Autonomy" section (level picker + per-domain overrides + the hatch behind a confirm). |
| **RN.7** | Checkpoint/rollback (§5.2) + capability-request channel (§5.3) — *may split out* if RN.0–RN.6 is already a full chapter. |

Smaller than it looks: RN.1–RN.2 are a composition layer over existing knobs
(the `[memory] profile` / `[access] level` pattern); RN.3 extends an existing
enum; the genuinely new surface is RN.5/RN.7.

---

## 10. Open questions (resolve in-phase, not blocking RN.0)

- **Domain taxonomy for overrides.** Is `domain` the tool-name first segment,
  the group base, or a curated set? Decide in RN.2 by what reads cleanly in a
  real `aivyx.toml`.
- **Batched-approval mechanics (`supervised`).** Where the queue lives and how
  the operator reviews a batch (a Studio inbox? a `aivyx autonomy review`?).
- **Per-channel autonomy.** Beyond Local, can a *specific* trusted webhook
  carry its own tier? Likely an `[[autonomy.override]]` keyed by channel later.
- **Hatch + teams.** Whether a Nonagon specialist can ever inherit the §6 hatch
  (default: **no** — attenuation strips it; the lead holds it, specialists don't).

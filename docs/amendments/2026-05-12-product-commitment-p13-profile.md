# Amendment A9 — Product Commitment P13: Assistant Profile

**Date:** 2026-05-12
**Phase:** 56
**Adds to:** PRODUCT.md — new Product Commitment 13
("Assistant Profile") inserted between P12 (Tools as
Separate Processes Over Daemon IPC) and the Status section.
**Implementing phases:** 57 (Foundation), 58 (Inspection) —
see ROADMAP.md.

---

## What's new

PRODUCT.md gains a thirteenth Product Commitment: **P13 —
Assistant Profile**. P13 establishes Profile as the
operator-declared, mostly-static identity layer of an Aivyx
instance. Profile is distinct from:

- the **role envelope** (P7 + P9), which gates *what the
  agent may do*; and
- **Persona** (P14, filed alongside this amendment as
  Amendment A10), which is the reflection-written *dynamic*
  identity layer.

Profile shapes *how the agent speaks and judges*. Roles
shape *what the agent may do*. Persona is *what the
agent's voice becomes* over time. The three compose; they
do not substitute.

---

## The new rule

> **Every Aivyx instance carries an operator-declared
> Profile that pins who this assistant is for and how it
> communicates. Profile is loaded once per daemon lifetime
> and injects into every turn's system prompt regardless of
> active role. Profile is mutable only by the operator, and
> only through operator-facing surfaces (CLI subcommands,
> Web UI panes); the agent cannot modify its own Profile.**

---

## What this commits us to

1. **Profile is a contract-level concept distinct from the
   role envelope.** A role's `system_prompt` field per P9
   may further specialize the agent's voice for that
   role's specific task, but Profile is the
   role-orthogonal identity layer. Switching roles does
   not switch Profiles.

2. **Profile is single-instance per operator.** Per P6
   (OS-Level Operator Identity) and P1 (Single Operator,
   Single Primary Agent), there is exactly one Profile
   per Aivyx daemon. There is no "switch profile" gesture
   parallel to "switch role." A second operator means a
   second OS user means a second daemon means a second
   Profile.

3. **Profile is operator-declared at install/init time
   and operator-mutable thereafter.** The agent cannot
   write to its own Profile. Profile changes are
   operator-driven through CLI or Web UI surfaces, *not*
   through the reflection layer (P8) or any in-turn
   mechanism. Reflection writes shape Persona (P14), not
   Profile.

4. **Profile is plain-text-inspectable.** Profile lives
   in a plain-text operator-readable file (most likely
   sharing the existing `aivyx.toml`, but the contract
   commits only to operator-readability, not the storage
   shape). The operator can read their Profile without
   unlocking the redb store, the same way they read role
   configurations today.

5. **Profile carries operator-declared identity fields
   in (at minimum) these categories:**
   - **Operator profile** — who the operator is, in
     enough detail that the agent can frame language for
     the operator's domain (role, expertise level,
     primary work context).
   - **Communication style** — operator preferences on
     verbosity, formality, citation frequency, source
     referencing, list-vs-prose, etc.
   - **Primary use cases** — the 1–3 use-case archetypes
     the assistant is being shaped around (e.g. *"Rust
     systems programming"*, *"personal-finance
     analysis"*, *"research synthesis"*). Drives default
     domain assumptions.
   - **Behavioral preferences** — non-capability
     defaults that flavor the agent's judgment
     (e.g. *"prefer integration tests over mocks"*,
     *"always cite sources when summarizing"*).
   - **Behavioral constraints** — non-capability
     guardrails that the agent should respect across
     every role (e.g. *"never autonomously commit
     code"*, *"always confirm destructive shell
     commands"*).
   - **Assistant name** — what the operator calls this
     specific assistant. Distinct from product name
     (*Aivyx*) and from role names (*coder*,
     *researcher*).

   The contract pins these **categories**, not the field
   names or the on-disk shape. Field-naming and shape
   are implementation choices for Phase 57 per the P9
   precedent (*"It does not pin the field names or the
   exact TOML shape"*).

6. **Profile injects into every turn's system prompt.**
   The system-prompt assembly pipeline at turn start
   composes Profile alongside (not inside) the
   role-derived envelope description. Both flow into the
   final prompt; Profile is not a sub-section of any
   single role.

7. **Profile carries no secrets.** Per the
   plain-text-inspectable property, Profile must not be
   used for API keys, passphrases, tokens, or any
   secret material. Secret storage remains the AEAD-
   encrypted redb store, and credential handling
   remains in `aivyx-config`'s existing credential
   types (e.g. `PassphraseSource`).

---

## What this commitment deliberately does not say

- **It does not pin the field names or the exact storage
  shape.** Per P9 precedent. Phase 57 chooses TOML vs JSON,
  flat vs nested, separate file vs section of `aivyx.toml`,
  exact field naming.

- **It does not require Profile to be encrypted.** Profile
  carries no secrets per Commit 7 above. The plain-text
  property is intentional — operators read their Profile
  without ceremony.

- **It does not require `aivyx init` to be the only entry
  point for Profile creation.** Phase 57 extends the init
  wizard (Phase 44) with use-case prompts, but a future
  channel adapter or Web UI surface may also bootstrap a
  Profile. The contract pins operator-declared, not
  init-wizard-declared.

- **It does not preclude Profile from referencing other
  state.** Profile may point at memory entries, role
  names, scheduled tasks; it just isn't *that* state. The
  pointer-vs-value distinction follows the existing
  precedent in role configs (where `parent_role` is a
  pointer, not embedded content).

- **It does not say there is exactly one system-prompt
  ordering.** Phase 57 makes the call on whether Profile
  precedes or follows the role-envelope description in the
  final system prompt. The contract pins *both flow into
  every turn*, not *in what order*.

- **It does not commit Profile to the audit chain.**
  Profile changes are operator actions taken outside the
  daemon's turn loop (CLI subcommands, Web UI panes).
  Whether those changes get audit entries is an
  implementation decision for Phase 58. The contract pins
  Profile mutability *to the operator*, not the
  observability of those mutations.

---

## How Profile composes with roles, capabilities, and Persona

- **Role envelope (P7 + P9)** gates capabilities. Profile
  does not. An operator cannot grant `shell.exec` through
  Profile; that's the role's job. Profile cannot escalate
  past any role's envelope; it has no envelope of its own.

- **System prompt (P9 dimension)** is the per-role prompt
  override. Profile injects alongside the role's
  `system_prompt`, not inside it. A role may further
  specialize the agent's voice for that role's task;
  Profile is the role-orthogonal identity layer that
  flavors every role.

- **Memory (G3)** is dynamic and observation-driven.
  Profile is static and operator-declared. Memory may
  capture facts that reinforce Profile (e.g. *"operator
  uses Vim, not VSCode"*), but Profile is the contract
  surface, memory is the substrate.

- **Persona (P14)** is the dynamic counterpart of
  Profile. Profile is the static seed; Persona is the
  evolving voice that grows from Profile + accumulated
  reflection-approved deltas. Profile is *who the
  operator declared the assistant to be*; Persona is
  *who the assistant becomes through operator-gated
  evolution*. Both inject at turn start; effective
  voice = Profile + sum(approved Persona deltas).

---

## Why this is a contract surface, not implementation

Profile shapes every turn's system prompt without role
attenuation. If Profile were left to implementation
discretion, a future phase could ship "Profile as a
memory topic" or "Profile as a role-config field," each
of which would silently couple identity to capability or
to dynamic state. Pinning Profile as a separate contract
surface prevents that drift — every implementation phase
that touches identity must respect Profile's separateness
from roles, memory, and Persona.

---

## Forward — what Phase 57 and Phase 58 will land

Per ROADMAP.md (committed 2026-05-12):

- **Phase 57 — Profile Foundation.** Storage shape decision,
  Profile struct location (probably `aivyx-config`, possibly
  a new substrate location depending on Q-block resolution),
  init-wizard extension with use-case prompts, system-prompt
  assembly composing Profile alongside the role-envelope
  description.
- **Phase 58 — Profile Inspection (closes P13).**
  `aivyx profile show` / `aivyx profile edit` CLI
  subcommands; Web UI Profile pane mirroring the CLI surface
  via the existing Query/QueryResponse IPC envelope (Phase
  47). After Phase 58 the operator has a fully-shaped
  static identity layer driving every turn.

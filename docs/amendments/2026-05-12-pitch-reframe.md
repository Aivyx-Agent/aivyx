# Amendment A8 — PRODUCT.md Pitch Reframe

**Date:** 2026-05-12
**Phase:** 56
**Supersedes:** Replaces the **one-line pitch** at the top of
PRODUCT.md (originally LOCKED 2026-04-15). The rest of
PRODUCT.md's text is unchanged by this amendment (P13 and P14
are added by separate amendments A9 and A10 in the same
Phase 56 batch).
**Implementing phase:** 56

---

## What changed

The original pitch sentence read:

> **Aivyx is a personal autonomous agent platform that runs on
> your hardware, talks to cloud LLMs under your own API key, and
> never compromises privacy or auditability for the sake of a
> feature.**

After Phase 55, the operator restated the vision more
specifically — Aivyx is being built as a self-learning,
self-improving AI-*personal-assistant* with a user-defined
Profile and Persona, not as a generic *"autonomous agent
platform."* The pitch is amended to reflect the narrower
target.

The new pitch is:

> **Aivyx is a self-learning, self-improving AI-personal
> assistant with a user-defined Profile and Persona, running
> on your hardware, talking to cloud or local LLMs under
> your own credentials, and never compromising privacy or
> auditability for the sake of a feature.**

---

## Why this is a narrowing, not a widening

The original pitch's spine — *runs on your hardware* +
*your own API key* + *never compromises privacy or
auditability* — is fully preserved. The amendment narrows the
**identity** of the agent (from a generic *platform* to a
*personal assistant*) and adds an explicit identity layer
(*user-defined Profile and Persona*) that the original pitch
did not name.

Three substantive shifts:

1. **"Personal autonomous agent platform" → "AI-personal
   assistant".** The original framing positioned Aivyx as a
   generic substrate for autonomous agents of any shape. The
   amended framing positions it as a personal-assistant
   product. The substrate is unchanged; the lens is
   narrowed.

2. **"Cloud LLMs … your own API key" → "cloud or local LLMs
   … your own credentials".** Phase 25 added OpenAI-compatible
   provider support and Phase 34 added first-class Ollama
   support. The pitch's exclusive reference to cloud LLMs was
   already inaccurate at Phase 34; this amendment corrects it.
   "Credentials" replaces "API key" because Ollama doesn't
   need one.

3. **"Profile and Persona"** — new contract surface,
   formally introduced by amendments A9 (P13 — Assistant
   Profile) and A10 (P14 — Persona), both filed in Phase 56
   alongside this amendment.

---

## What this amendment deliberately does not say

- **It does not say the agent stops being autonomous.** The
  word "autonomous" is dropped from the pitch sentence because
  it overlapped with "agent" — but G5 (Autonomous and
  scheduled execution) is unchanged, and missions per P2 still
  run autonomously between gates.

- **It does not say the substrate-only-core principle changes.**
  P10's eight-tools cap and the substrate / infrastructure /
  third-party taxonomy are unchanged. The agent's *identity*
  layer (Profile + Persona) is orthogonal to its *tool*
  surface.

- **It does not say roles are subsumed by Profile.** Per A9,
  Profile is the identity-layer concept; roles remain the
  capability-layer concept per P7 and P9. Profile flavors
  *how the agent speaks and judges*; roles gate *what the
  agent may do*. They compose; they don't substitute.

---

## Why now

Phase 55 closed with the operator's vision statement
(*"self-learning, self-improving AI-personal assistant with
a user-defined Profile and Persona based on the end-user
use-case"*) and the project pivoting to the Profile + Persona
forward arc (Phases 56–60, scaffolded in ROADMAP.md
2026-05-12). Filing the pitch amendment alongside A9 and A10
keeps PRODUCT.md self-consistent — the pitch and the
commitments it gestures at land in the same phase.

# Aivyx — Vision & North Star

> **Mission: Build It Right First.**
>
> This document sits *above* the per-product contracts. [`PRODUCT.md`](PRODUCT.md)
> and [`DESIGN.md`](DESIGN.md) say **how** the assistant works and what it
> commits to; this says **what** Aivyx is, **why** it exists, and **in what
> order** it grows. Every chapter is held against it. When a build can't answer
> the [guiding test](#the-guiding-test), it isn't ready.

---

## 0. The mission — Build It Right First

Aivyx was attempted once before and **got away from us** — not because the
vision was wrong, but because it reached for the cathedral (an enterprise OS, a
network of agents) before the foundation could hold weight. This rebuild exists
to invert that mistake: **a correct, solid foundation first, and every layer
above it earned, in order.**

There is **no deadline.** Correctness, clarity, and durability beat speed every
time. A thing built right is a thing that compounds; a thing rushed is a thing
that gets away from us again. "Build It Right First" is not a slogan — it is the
tie-breaker for every decision in this repository.

## 1. What Aivyx *is* (and is not)

**Aivyx is a self-learning agentic *personal* assistant that the end user makes
their own.** A user-defined **Profile**, a reflection-grown **Persona/Soul**, a
local-first encrypted substrate, a capability-gated security model, and a
multi-agent team it can convene to do real work. It is meant to become
*yours* — an irreplaceable partner that improves through use, governed so that
it can never rewrite who it is without you.

**Aivyx is not** a faceless worker farm. The self-learning identity — the Soul,
the governed Persona evolution, the guarantee that *no agent reshapes itself or
another without the operator* — is not decoration. It is the heart of the
product and the seed of its moat. The architecture has a heart, and it points
at *the personal*. We build with that grain, never against it.

## 2. The architecture's destination — Nonagon becomes Nexus

The agent does not work alone. It convenes a **Nonagon team**: a lead delegating
to capability-**attenuated** specialists, gated, audited, message-passing — all
*inside one operator's trust boundary*.

The long-term vision is a **network of agents** — discovering each other,
sharing learned skills, growing together, and (with their operators' consent)
delegating work to one another. The crucial insight, and the reason this is
reachable rather than aspirational:

> **A network of agents is a Nonagon team with the trust boundary moved.**
> Same delegation, same attenuation, same gates, same audit — the *only* new
> thing is identity and trust at the boundary between operators.

So **everything we build toward excellent local teams is already building toward
the network.** The local team is the rehearsal. We design every team primitive —
delegation, messaging, skill-sharing, capability attenuation — as if the peer on
the other side *might one day be a stranger's agent on another machine.* That
foresight is the cheapest insurance "no deadline" buys us, and the difference
between an extension and a second rewrite.

### The keystone we get right early

Almost everything can be built lazily, in order. **One primitive cannot:**
**agent identity and cross-boundary trust.** Everything today assumes *"I trust
my operator."* The moment a second agent exists, the model must become *"I trust
my operator — and I trust no peer by default; peers earn scoped, attenuated,
revocable trust, and any content from them (a shared skill, a request) is
sandboxed and provenance-tracked."* Trust and identity are the one thing that
**cannot be safely retrofitted**, so their *shape* is a design constraint from
now, even if the network itself is built last. That shape is specified in
[`docs/FEDERATION.md`](docs/FEDERATION.md) — the keystone design.

## 3. The privacy line that makes it possible

The core promise is sovereignty: **your data stays on your machine.** A network
of agents only honors that promise if the line is razor-sharp:

> Agents share **procedures, capabilities, reputations, public personas** —
> **never** the operator's private memory or data.

Our model already draws this line. A learned skill is a *procedure*
(`{name, trigger, procedure}`); the private memory it was learned from never has
to travel. *What an agent knows how to do* is shareable; *what an agent knows
about its operator* is not. This boundary is not a feature to add later — it is
a constraint every shareable artifact is designed under from the start.

## 4. Open-core, by layers — not by forks

Aivyx is **open-core**. The personal assistant is free and open: it is the
foundation, the trust, and the adoption engine. Capabilities that serve teams,
domains, and organizations are built **as layers and packs on the one engine**,
behind a stable SDK boundary — **never as a forked second codebase.** A fork
means maintaining two of everything and watching them drift; the boundary gives
us the separation we need without the tax we can't afford. The agent engine
stays *one engine*.

## 5. The order of growth (the chain)

The layers form a **dependency chain — each link earns and funds the next:**

```
Open PA core            — solid, excellent, in users' hands        [the foundation]
  → domain packs        — the assistant becomes a specialist team  [each = a Nonagon team]
    → distribution      — a place to find and adopt them
      → multi-agent     — agents that span machines (the keystone) [identity + trust]
        → the network   — agents that discover, share, and grow together
```

We walk this **in order.** The previous attempt jumped to the last links before
the first was solid; this one does not. And crucially, **every link is
independently valuable and reversible** — a great open PA with a few excellent
domain packs is already a real, worthy thing. We are never one-build-away from
worthless; we are always shipping something whole.

## 6. The guiding test

Before any chapter opens, it answers three questions. If it can't, it waits:

1. **Does it move us along the chain** (or strengthen a link we already hold)?
2. **Is it Nexus-ready** — does it respect the trust boundary, the privacy line,
   and the "no operator-bypass" invariant, so it extends to a network of agents
   rather than fighting it later?
3. **Is it built right** — correct, tested, audited, and clear enough that a
   future reader (including us) trusts it without re-deriving it?

## 7. Invariants (the lines we do not cross)

- **Build it right beats ship it fast.** No deadline; correctness is the
  tie-breaker.
- **One engine.** Commercial value is layers and packs behind an SDK boundary,
  never a forked codebase.
- **The operator is sovereign.** No agent widens its own reach or rewrites its
  own — or another's — identity without the operator. (PRODUCT.md P8, extended
  to the network.)
- **Procedures travel; data does not.** The privacy line holds at every boundary.
- **Trust is earned and scoped, never assumed.** No peer agent is trusted by
  default; capability attenuation holds *across* operators, not just within a team.
- **Every stage stands alone.** We never bet the whole on a link we haven't
  reached.

---

*The commercial strategy that rides on this — how the layers are licensed, sold,
and sustained — is maintained privately, outside this public repository. This
document is the part that belongs to everyone who builds on Aivyx: the mission,
the destination, and the discipline.*

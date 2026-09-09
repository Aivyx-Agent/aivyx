# Amendment A5 — Substrate Tool Count Update

**Date:** 2026-04-20
**Phase:** 38
**Supersedes:** Narrows P10 (Product Commitment 10 —
Substrate-Only Core, Seven Tools Forever). The word "seven"
becomes "eight" and `web.post` is added to the enumerated
list. All other P10 text is unchanged.
**Implementing phase:** 37

---

## What changed

Phase 37 added `web.post` — a separate `Tool` with its own
`Tool::name()` ("web.post"), its own scope base (`net.post`),
and its own registration in the binary. By P10's own
taxonomy, `web.post` is substrate (operator-facing, not
self-management), which means the seven-tool count is now
eight.

---

## Why this is substrate, not infrastructure

P10's three-tier taxonomy (substrate / infrastructure /
third-party) defines infrastructure as "tools the agent uses
to manage itself" — reflection, missions, role switching.
`web.post` is not self-management. It sends HTTP requests to
external services on the operator's behalf, the same category
as `web.fetch`.

The `net.post` scope base has been in the substrate base
table since Phase 0 (D4's original design, documented in
Amendment A3). The base was always anticipated; Phase 37
merely shipped the tool that exercises it.

---

## The amended rule

> **Aivyx PA core ships exactly eight first-party tools forever:
> `fs.read`, `fs.write`, `memory.read`, `memory.write`,
> `memory.forget`, `shell.exec`, `web.fetch`, `web.post`.
> Adding to or removing from this list requires a
> `PRODUCT.md` amendment.**

The rest of P10 — the substrate-only principle, the
substrate/infrastructure/third-party taxonomy, the "what
this does not say" section — is unchanged.

---

## Why eight and not "consolidate into web.fetch"

`web.post` could have been implemented as a mode of
`web.fetch` (a `method` field accepting GET/POST/PUT/etc.).
Phase 37 chose separate tools for structural security
reasons:

1. **Different scope bases.** `net.fetch` (read) vs
   `net.post` (write). The capability system can gate
   read-vs-write independently.
2. **Different trust tiers.** `net.fetch` is
   SemiTrusted-accessible (a Telegram researcher agent can
   fetch URLs). `net.post` is Trusted-only (write verbs
   require direct operator trust).
3. **Different input shapes.** POST needs `body`,
   `content_type`, `method` fields that don't apply to GET.

A single tool with a mode flag would require runtime
branching on trust tier and input validation, and would blur
the scope boundary. Two tools make the security boundary
structural.

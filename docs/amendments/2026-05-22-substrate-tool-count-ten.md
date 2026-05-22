# Amendment A11 — Substrate Tool Count Update (Eight → Ten)

**Date:** 2026-05-22
**Phase:** 100
**Supersedes:** Narrows P10 (Product Commitment 10 —
Substrate-Only Core, Eight Tools Forever). The count "eight"
becomes "ten" throughout P10's text and `fs.delete` +
`fs.metadata` are added to the enumerated list. P10's
substantive text — the substrate-only principle, the
three-tier substrate/infrastructure/third-party taxonomy, and
the "what this does not say" section — is unchanged.
**Implementing phase:** 100

---

## What changed

Phase 100 added two first-party tools: `fs.delete` (delete a
file or empty directory) and `fs.metadata` (stat a path —
size, type, modified time, permissions; on a directory,
list its entries). Each is a separate `Tool` with its own
`Tool::name()` ("fs.delete" / "fs.metadata"), its own scope
base (`fs.delete` / `fs.metadata`), and its own registration
in the binary. By P10's own taxonomy both are substrate
(operator-facing filesystem operations, not agent
self-management), which means the eight-tool count is now
ten.

---

## Why these are substrate, not infrastructure

P10's three-tier taxonomy (substrate / infrastructure /
third-party) defines infrastructure as "tools the agent uses
to manage itself" — reflection, missions, role switching.
`fs.delete` and `fs.metadata` are not self-management. They
operate on the operator's filesystem on the operator's
behalf, in exactly the same category as `fs.read` and
`fs.write` — the substrate's read/write filesystem pair now
becomes a read/write/delete/inspect quartet.

The `fs.delete` and `fs.metadata` scope bases have been in
the substrate base table since Phase 0 (D4's original
design, documented in Amendment A3, both with the `path
glob` qualifier kind). The bases were always anticipated;
Phase 100 merely shipped the tools that exercise them.

---

## The amended rule

> **Aivyx core ships exactly ten first-party tools forever:
> `fs.read`, `fs.write`, `fs.delete`, `fs.metadata`,
> `memory.read`, `memory.write`, `memory.forget`,
> `shell.exec`, `web.fetch`, `web.post`. Adding to or
> removing from this list requires a `PRODUCT.md`
> amendment.**

The rest of P10 — the substrate-only principle, the
substrate/infrastructure/third-party taxonomy, the "what
this does not say" section — is unchanged.

---

## Why two tools and not folded into `fs.read` / `fs.write`

`fs.delete` and `fs.metadata` could have been implemented as
modes of the existing filesystem tools (a `delete` flag on
`fs.write`, a `stat` flag on `fs.read`). Phase 100 chose
separate tools for the same structural-security reasons
Amendment A5 gave for keeping `web.post` separate from
`web.fetch`:

1. **Different scope bases.** `fs.read`, `fs.write`,
   `fs.delete`, and `fs.metadata` are four distinct D4
   bases. The capability system gates read, write, delete,
   and inspect independently — a role can hold `fs.read`
   without ever being able to delete.

2. **Different trust tiers.** `fs.delete` is destructive.
   It is registered behind a `shell.exec`-style
   registration-time trust gate: Local channels only,
   absent entirely from a SemiTrusted dispatch registry.
   `fs.metadata` is read-only and carries no such gate. A
   single tool with a mode flag would have to branch on
   trust tier at runtime; two tools make the boundary
   structural.

3. **Different input and output shapes.** `fs.delete`
   takes a path and returns a deletion result;
   `fs.metadata` takes a path and returns a stat record
   (or, for a directory, an entry list). Neither shape
   maps cleanly onto `fs.read`'s "return file contents"
   or `fs.write`'s "accept file contents."

`fs.metadata` deliberately absorbs directory listing rather
than spawning a separate `fs.list` tool and scope: a
directory's metadata *is* its entry list, no new D4 base is
needed, and the substrate stays at a clean ten.

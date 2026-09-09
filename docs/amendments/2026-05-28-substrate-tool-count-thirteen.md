# Amendment A12 — Substrate Tool Count Update (Ten → Thirteen)

**Date:** 2026-05-28
**Phase:** 109
**Supersedes:** Narrows P10 (Product Commitment 10 —
Substrate-Only Core, Ten Tools Forever, post-A11). The count
"ten" becomes "thirteen" throughout P10's text and
`git.status`, `git.diff`, and `net.dns` are added to the
enumerated list. P10's substantive text — the substrate-only
principle, the three-tier substrate/infrastructure/third-party
taxonomy, and the "what this does not say" section — is
unchanged.
**Implementing phase:** 109

---

## What changed

Phase 109 added three first-party tools:

- **`git.status`** — runs `git status --porcelain` against a
  configured repo path and returns parsed entries.
- **`git.diff`** — runs `git diff` (optionally `--cached`,
  optionally scoped to a file or directory) against a
  configured repo path and returns the unified-diff output.
- **`net.dns`** — resolves a hostname to one or more IP
  addresses via `tokio::net::lookup_host`.

The two git tools share a new `git.read` capability scope base
qualified by repo path; the DNS tool uses the existing
`net.dns` scope base that has lived in `KNOWN_BASES` since
Phase 0 (it was one of the eight declared-but-toolless scopes
Phase 100's audit catalogued for "tool later" treatment).

By P10's own taxonomy all three are substrate (operator-facing
read operations against operator-owned resources, not agent
self-management), which means the ten-tool count post-A11 is
now thirteen.

---

## Why these are substrate, not infrastructure

P10's three-tier taxonomy (substrate / infrastructure /
third-party) defines infrastructure as "tools the agent uses
to manage itself" — reflection, missions, role switching.
`git.status`, `git.diff`, and `net.dns` are not
self-management. They operate on the operator's repos and the
operator's network on the operator's behalf, in exactly the
same category as `fs.read`, `fs.write`, and `net.fetch`. The
substrate's read primitives now grow from filesystem +
HTTP-fetch to filesystem + HTTP-fetch + git-repo-read +
DNS-resolve.

The `net.dns` scope base has been in the substrate base table
since Phase 0 (D4's original design, documented in Amendment
A3). The base was always anticipated; Phase 109 merely shipped
the tool that exercises it.

`git.read` is a new scope base — the first since A11's
`fs.delete` / `fs.metadata` additions at Phase 100. It joins
`KNOWN_BASES` in `aivyx-capability` at the same edit. The
qualifier kind is `PathGlob` (matching `fs.read`'s
qualifier pattern).

---

## The amended rule

> **Aivyx PA core ships exactly thirteen first-party tools
> forever: `fs.read`, `fs.write`, `fs.delete`, `fs.metadata`,
> `memory.read`, `memory.write`, `memory.forget`,
> `shell.exec`, `web.fetch`, `web.post`, `git.status`,
> `git.diff`, `net.dns`. Adding to or removing from this list
> requires a `PRODUCT.md` amendment.**

The rest of P10 — the substrate-only principle, the
substrate/infrastructure/third-party taxonomy, the "what this
does not say" section — is unchanged.

---

## Why one shared `git.read` scope, not two separate

`git.status` and `git.diff` could have been gated by separate
`git.status` / `git.diff` scope bases — that's how `fs.read`
and `fs.write` are split, and how `fs.delete` and
`fs.metadata` were split at A11. Phase 109 chose **one
shared scope base** for the git pair for two reasons:

1. **Different orthogonality.** `fs.read` and `fs.write` gate
   substantively different operations (one is non-destructive
   read, one is destructive write). `git.status` and
   `git.diff` are both read-only inspection of the same repo —
   the natural capability grant is "this role can read this
   repo," not "this role can run `git status` but not `git
   diff`." Splitting the scope would invite operators to grant
   `git.status` without `git.diff` in cases where the
   read-only invariant means either-both-or-neither is
   actually what they want.
2. **Future git tool additions stay capability-compatible.**
   If a future phase adds `git.log` (also read-only inspection
   of the same repo), it joins `git.read` without a fresh
   scope base. The pattern mirrors how `memory.read`,
   `memory.write`, and `memory.forget` would (counterfactually)
   not share — they wouldn't, because they're distinct
   operations on different invariants. `git.*` read tools all
   share the read invariant; that's the right grouping.

A future destructive git tool (`git.commit`, `git.checkout`,
`git.reset --hard`) would warrant a separate `git.write`
scope. Phase 109 ships read-only only; the question doesn't
yet arise.

---

## Why `net.dns` lands without an amendment for the scope itself

The `net.dns` scope base existed in `KNOWN_BASES` from Phase
0 — D4's original substrate-base inventory included it,
documented in Amendment A3. Phase 100's audit listed it as
"tool later" alongside seven other declared-but-toolless
scopes. Phase 109 ships the tool. No amendment is needed for
the scope base; A12's count change from ten to thirteen
covers the new substrate tool that exercises it.

The other seven declared-but-toolless scopes stay deferred:
- `shell.spawn` — long-running process spawn; distinct from
  `shell.exec`'s short-lived shape. Phase-later.
- `audit.read` — agent-facing read of the audit chain.
  Overlaps with `aivyx-pa audit export` (Phase 105) but at the
  agent layer; phase-later if pressure surfaces.
- `config.read` / `config.write` — agent introspection /
  mutation of `aivyx-pa.toml`. Substrate-design adjacent;
  phase-later.
- `audit.read`, `shell.spawn` are the most likely
  near-term candidates. The others may stay reserved.

A12 is silent on these — they're still declared-but-toolless
after Phase 109, and the next phase that ships any of them
files A13 (or extends A12-via-addendum, depending on the
shape).

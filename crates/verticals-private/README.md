# Private vertical packs

This directory holds **commercial / private vertical packs** — the paid half of
the Aivyx ecosystem. Everything here except this `README.md` and the
`.gitignore` is **git-ignored**: private packs build inside the monorepo but
never land in the public (BUSL-1.1) repo.

## How it works

- **Auto-membership.** The root `Cargo.toml` lists `crates/verticals-private/*`
  as a workspace member glob. Drop a pack crate here
  (`crates/verticals-private/aivyx-acme/`) and it joins the workspace
  automatically, sharing the core's lockfile and `target/`. Cargo silently
  ignores glob matches that have no `Cargo.toml`, so an empty (fresh-clone)
  private dir builds without complaint.
- **Same contract as the open example.** A private pack is structurally
  identical to the open [Kitchen pack](../verticals/aivyx-kitchen) — a
  team-config crate and/or a toolkit tool-process crate, each depending on
  **`aivyx-vertical-sdk` alone** for shipping code (path: `../../aivyx-vertical-sdk`,
  the same depth as the in-tree packs). Follow the step-by-step build in
  [`docs/VERTICAL_PACKS.md` §6](../../docs/VERTICAL_PACKS.md#6-build-your-own-pack--step-by-step).
- **What stays public.** The *only* thing a pack adds to the public tree is its
  additive scope bases in `aivyx-capability::KNOWN_BASES`. The domain logic,
  team souls, and tools stay private here. (If a pack's scopes must also stay
  secret, that is a future packaging decision — today bases live in the public
  capability crate.)

The boundary is deliberate: open core + open example pack, commercial packs
behind the same stable SDK seam. See the [ecosystem notes](../../docs/VERTICAL_PACKS.md#2-the-pack-format).

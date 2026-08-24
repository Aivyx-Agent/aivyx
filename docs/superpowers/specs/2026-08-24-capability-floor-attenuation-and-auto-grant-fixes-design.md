# Capability-Floor Attenuation and Auto-Grant Fixes Design

## Motivation

The final whole-branch review of the vertical-pack-aware-capability-floor
work (`docs/superpowers/specs/2026-08-24-vertical-pack-aware-capability-floor-design.md`,
range `672dcdc0..4c04e1cc`) confirmed the original Critical finding — real
vertical packs breaking — is genuinely closed (independently re-verified
against the actual shipped `crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml`
file, not a synthetic fixture). But it found two new Critical issues in the
same diff, both requiring real design work rather than a mechanical patch.
This design closes both, on the same branch, before merge.

## C1: `bind_lead_scopes`'s unification breaks NT-02 attenuation downstream

### The bug, traced fresh

There are two separate, independent places NT-02 ("a specialist can never
exceed its lead") gets enforced, not one:

1. `bind_lead_scopes` (`crates/aivyx-channel/src/team_mission_driver.rs`)
   sets each `TeamConfig` member's own `capability_scopes` field to
   `raw_floor ∩ own_declared_bases` (plus orchestration markers) — this is
   what the vertical-pack-aware-capability-floor plan's Task 2 unified
   across lead and specialist.
2. Downstream, `crates/aivyx-team/src/attenuation.rs`'s
   `attenuate_for_member(lead: &CapabilitySet, declared: &[Scope]) ->
   CapabilitySet` independently re-computes each specialist's **effective
   runtime** capabilities as `declared ∩ lead.grants(...)`, called from
   `SpecialistFactory::build` (`crates/aivyx-team/src/factory.rs:146-151`),
   itself called from `SpecialistPool::run`
   (`crates/aivyx-team/src/pool.rs:258`) with `&self.lead_caps` — a value
   set once at `SpecialistPool::new` construction, which
   `TeamAssembly::build` (`crates/aivyx-team/src/assembly.rs:52-99`)
   receives as its own `lead_caps: CapabilitySet` parameter and passes
   straight through.

Both real call sites of `TeamAssembly::build` — `run_mission`
(`crates/aivyx-cli/src/bin/aivyx_modules/team.rs:229`) and
`assemble_runtime` (`crates/aivyx-channel/src/team_mission_driver.rs:1473`,
`:1498-1509`) — pass `lead.declared_capabilities()` (the lead **member's**
own, now-narrowed, post-`bind_lead_scopes` field) as this parameter.

Before the vertical-pack-aware-capability-floor plan's Task 2, the lead's
own field *was* unconditionally the full raw floor, so this double-check
was harmless (idempotent — `attenuate_for_member`'s ceiling always equaled
the raw floor anyway). Task 2's fix correctly narrows the lead's own field
to `floor ∩ its own declared bases` (closing the earlier "widening"
finding) — but since `TeamAssembly::build` reuses that *same* narrowed
value as `attenuate_for_member`'s ceiling, a purely-orchestration lead
(`default_nonagon`'s coordinator, correctly declaring only `[memory.read,
memory.write, team.delegate]` — its own soul: "you never execute domain
work directly") now caps every specialist's **effective** capabilities at
that narrow set too. Empirically A/B-verified by the final reviewer: every
specialist's effective capability set collapses to `team.message` only —
reproducing, verbatim, the "missions report done but do nothing" bug
`bind_lead_scopes` was originally built to fix.

### Fix

`TeamAssembly::build`'s `lead_caps` parameter has exactly one real
consumer: `SpecialistPool`'s NT-02 ceiling (`TeamAssembly::lead_caps()`,
the public accessor, has zero callers anywhere in the codebase — confirmed
by grep). The lead's own `ConcreteAgent` is built **separately**, at both
real call sites, from its own already-held local variable — it never reads
this value back from the assembly. So the fix doesn't need new parameters
threaded through the pipeline; it needs the *value* passed for the
existing parameter corrected, plus a rename so this exact conflation can't
recur silently:

- Rename `TeamAssembly::build`'s `lead_caps: CapabilitySet` parameter (and
  the field `SpecialistPool` stores it in) to `ceiling: CapabilitySet`,
  with the doc comment corrected: this is the operator's real, un-narrowed
  authority that bounds every specialist — not the lead's own operational
  capabilities, which happen to have been the same value only because the
  lead used to be granted the whole floor unconditionally.
- At both call sites, build `ceiling` from the raw floor they already hold
  (`lead_scopes: &[String]` in `run_mission`; `deps.lead_scopes` in
  `assemble_runtime`) via `CapabilitySet::from_scopes(lead_scopes.iter()
  .filter_map(|s| Scope::parse(s)))`, instead of reusing
  `lead.declared_capabilities()`.
- The lead's own `ConcreteAgent` construction is untouched — it keeps using
  `lead.declared_capabilities()` (the narrowed value), exactly as today.

Net effect: `bind_lead_scopes`'s specialist-branch computation (`raw_floor
∩ own_bases`, already correct) and `attenuate_for_member`'s downstream
re-check now agree on what "the floor" means. The second check becomes a
harmless, idempotent confirmation rather than a silent double-narrowing —
NT-02 stays enforced at two layers (not reduced to one), just consistently.

## C2: `compute_backcompat_floor`'s generic sweep needs a real opt-in, not an exclusion list

### The bug, audited fresh

The generic sweep (`crates/aivyx-cli/src/bin/aivyx.rs`) grants the bare
scope base of any tool registered in `tool_list`, except for an
`EXPLICIT_BASES` list covering only `fs.read/fs.write/fs.metadata/
fs.delete/shell.exec/net.fetch/net.post/workspace`. Since roughly 25
built-in tools register unconditionally regardless of operator config, the
floor now includes bases well beyond the three special cases (Ollama/MCP/
`applications`) this generalization was meant to subsume.

A systematic audit against the pre-branch hardcoded floor (`git show
672dcdc0:crates/aivyx-cli/src/bin/aivyx.rs`) confirms **five** bases that
reverse explicit, documented "never auto-grant this" decisions — one more
than the final review's own spot-check found:

- `git.write` — a *surviving* comment elsewhere in the same file still
  says "the write scope is role-config-driven, not auto-granted in the
  backcompat floor." Bare `git.write` + D4 Rule 2 = commit rights to every
  configured repo.
- `git.read` — the pre-branch code builds a `_git_read_scope` value and
  explicitly never uses it, underscore-prefixed, with a comment stating
  the Local CLI's capability set grants git access only "through the
  role's own `git.read:**`... declarations" — an operator must opt in
  explicitly even though `[git] repos` being configured is itself a real
  signal. (This one wasn't in the final review's own spot-check.)
- `role.update` — deleted comment: "Self-escalation scopes... deliberately
  NOT granted — P8 no-self-escalation."
- `reflection.apply` — deleted comment: `reflection.propose` was granted
  specifically because "a proposal only ever lands as Pending... this is
  the propose half, not self-modification." `reflection.apply` is the
  other half.
- `skills.write` — deleted comment: "the write half remains auto-proposer
  / role-declared." `skills.write` is the identity-modifying persona-chain
  writer.

Not every newly-exposed base is a regression, though — at least one
(`net.dns`) has its own comment in the pre-branch code already framing it
as behaving "like `net.fetch`" (a safe network-read), suggesting it was
likely an accidental pre-existing floor gap rather than a deliberate
withhold, matching the pattern this same file's comments document five
other times before this session. This is exactly why a blanket revert
would be the wrong fix, and why "is this tool present in `tool_list`" was
never a safe proxy for "should its scope auto-grant" in the first place —
the pre-branch code deliberately granted only the *safe half* of what some
already-unconditionally-registered tools need (`reflection.propose` yes,
`reflection.apply` no, from the *same* tool family).

### Fix: a real per-tool opt-in on the `Tool` trait

Add one method to `Tool` (`crates/aivyx-core/src/lib.rs`), defaulting to
`false` — fail-closed, matching this codebase's existing convention for
`Tool::mutates_outside_session()`:

```rust
/// Whether this tool's required scope may be auto-granted to the
/// default, floor-only role via the operator's backcompat floor.
/// Default `false`: a tool must explicitly opt in. Most tools should
/// NOT override this — third-party/OAuth integrations, anything
/// touching git.write, role self-modification, or persona/skill
/// identity writes stay withheld by default, requiring an operator to
/// declare them explicitly in a custom role's own `capability_scopes`.
fn auto_grantable_in_backcompat_floor(&self) -> bool {
    false
}
```

There are 100+ `impl Tool` sites in this codebase, but almost all of them
— every Gmail/Drive/Notion/Obsidian/N8N/Contacts/Calendar integration
tool, and every domain tool proxied through a toolkit-specific type —
were never in the floor before this branch and correctly get the safe
default with zero code changes. Exactly **seven** concrete types need an
explicit `true` override, matching precisely what's correctly granted
today:

- `aivyx_tool::proxy::ToolProxy` (`crates/aivyx-tool/src/proxy.rs`) —
  covers every tool-process-sourced tool (kitchen, `applications`, any
  future vertical toolkit): the operator configuring the `[[tool_process]]`
  entry *is* the opt-in.
- The three MCP proxy types in `aivyx-mcp` (`proxy.rs`, `resource_proxy.rs`,
  `prompt_proxy.rs`) — the operator configuring the `[[mcp_server]]` entry
  is the opt-in.
- The three Ollama tool structs in `crates/aivyx-channel/src/ollama_tools.rs`
  (`OllamaListTool`, `OllamaShowTool`, `OllamaPullTool`) — gated on
  `provider = "ollama"`, itself an explicit operator choice.

`git.write`/`git.read`/`role.update`/`reflection.apply`/`skills.write`
simply never override the method — withheld with zero new denylist code.
The deleted rationale comments are restored as doc comments on the trait
method itself (the general policy) and, where a specific tool's own
withholding needs its own note (e.g. `GitWriteTool`, `RoleUpdateTool`), at
the tool's own definition site.

**The call-site filter, not the function signature, does the gating.**
`compute_backcompat_floor` keeps its existing, already-tested `&[Scope]`-
shaped `tool_scope_bases` parameter unchanged — only its one call site in
`aivyx.rs` changes:

```rust
let tool_scope_bases: Vec<Scope> = tool_list
    .iter()
    .filter(|t| t.auto_grantable_in_backcompat_floor())
    .map(|t| t.required_scope(&serde_json::json!({})))
    .collect();
```

`EXPLICIT_BASES` inside `compute_backcompat_floor` stays exactly as it is
— it's a *separate*, additional layer about scope **shape** (a bare grant
of a path-qualified family like `fs.read` is dangerous regardless of
whether a tool considers itself auto-grantable), not about governance
opt-in. Both mechanisms are needed together; neither replaces the other.

## Testing

**C1:**
- New test proving the actual bug end-to-end: a `default_nonagon`-shaped
  config (coordinator declaring only orchestration scopes), a broad raw
  floor, run through the real `bind_lead_scopes` → `TeamAssembly::build` →
  `SpecialistFactory::build`, asserting a specialist's **effective**
  capabilities (the `CapabilitySet` `attenuate_for_member` actually
  returns, not `member.capability_scopes`) retain what the raw floor
  grants for its own declared bases. Mutation-proof: must fail if
  `ceiling` is reverted to `lead.declared_capabilities()`.
- Existing `factory.rs` test `build_attenuates_capabilities_against_the_lead`
  must keep passing unchanged — it exercises `attenuate_for_member`
  directly and isn't affected by which value callers pass, only by the
  function's own (untouched) logic.
- Both real call sites get a test (or an existing one extended) confirming
  `ceiling` is built from the raw floor parameter, not the narrowed lead
  field.

**C2:**
- New test: a fake `Tool` that does *not* override the new method
  contributes nothing to `compute_backcompat_floor`'s output, even though
  its required scope isn't in `EXPLICIT_BASES` — the mutation-proof for
  the regression itself (must fail if the call-site filter is removed).
- New test: a fake `Tool` that does override (`true`) contributes its
  scope normally — proving the opt-in path still works (this is what
  keeps kitchen packs working).
- `compute_backcompat_floor`'s own two existing pinning tests are
  unaffected by this change (their `tool_scope_bases: &[Scope]` fixtures
  stay exactly as they are) — but the branch's own end-to-end integration
  test (`compute_backcompat_floor_flows_a_configured_verticals_domain_scopes_through_bind_lead_scopes`,
  added during the vertical-pack-aware-capability-floor plan's own fix
  wave) needs its `tool_scope_bases` fixture reconsidered: it currently
  passes bare `Scope::parse("kitchen.read")` values directly, bypassing
  the new call-site filter entirely (since that filter lives in `aivyx.rs`'s
  real call site, not inside the function the test calls directly). This
  test should be updated to go through the real call-site logic (or a
  second, call-site-level integration test added) so it continues to
  prove what it claims post-fix.

## Out of scope

- No change to `EXPLICIT_BASES`'s own contents or mechanism.
- No change to `bind_lead_scopes`'s own unification logic (Plan 2 Task 2's
  fix for the lead-widening finding stays exactly as shipped) — C1's fix
  is entirely in how its output gets *consumed* downstream.
- No audit of the ~15 other newly-exposed bases beyond the five confirmed
  violations and the one confirmed-safe case (`net.dns`) — the new
  opt-in mechanism makes this audit unnecessary going forward (every tool
  defaults to withheld unless a maintainer explicitly reviews and opts it
  in), so there's no need to individually clear the remainder before this
  merges. Any of them that are genuinely useful and safe can be opted in
  later, one at a time, with its own rationale.
- No changes to the Tool trait beyond the one new default method — no
  other trait surface, no changes to `ToolExecutor::dispatch` or the
  permission-gate logic.

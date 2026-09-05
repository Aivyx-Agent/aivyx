# Chapter Picket Finding 3 (config-knob half): Operator Control Over the Injection Tripwire

**Status: approved, ready for implementation planning.**

## Context

PHASE_196.md logged Finding 3 as deliberately deferred: the injection
marker list was tuned for a coding agent's file/web content and had
never been evaluated against Gmail/Calendar/Slack/MCP traffic, with no
operator-facing config knob to disable or tune the tripwire. Phase 198
closed the *coverage* half (28 productivity-integration tools now get
scanned, where none were before) and explicitly deferred the
*config-knob* half as its own follow-up. This spec closes that
remaining half.

The tripwire is Chapter Picket's active scan
(`aivyx_injection_guard::scan_for_injection_markers`, invoked as
`check_for_injection` in `crates/aivyx-core/src/agent.rs`'s
`run_tool_call`, ~line 1410), layered on top of Chapter Bulwark's
passive fencing (`fence_untrusted_output`) at the same call site. Both
currently run unconditionally for every tool that declares
`output_is_untrusted() == true`.

## Scope

Two independent config knobs, one phase:

1. **`[agent] injection_scan_enabled`** (bool, default `true`) — global
   on/off for the active scan.
2. **`[agent] injection_scan_exempt`** (list of tool names, default
   empty) — per-tool exemption from the active scan, for an operator
   who wants the scan on everywhere except a specific noisy tool
   (e.g. a productivity integration whose real traffic keeps
   false-positiving) rather than turning it off entirely.

Bulwark's passive fencing is **not** gated by either knob — it runs
unconditionally for every `output_is_untrusted() == true` tool, exactly
as today. Turning either knob off only stops the *active scan and
possible turn escalation*; the model still sees untrusted content
marked as data, not instructions.

Explicitly out of scope, per the user's own scoping decision: any
special audit-log visibility or startup log line when either knob is
used — this matches every comparable safety knob in the codebase today
(`require_enforcement`, `guard_sensitive_paths`, `allow_private_egress`,
`tool_allowlist`), none of which get special logging beyond being a
readable config value. Also out of scope: expanding or editing
`INJECTION_MARKERS` itself (a separate, already-logged follow-up from
Chapter Picket) and a dedicated CLI subcommand (TOML-only, matching
`guard_sensitive_paths`/`allow_private_egress`'s precedent — no
existing plain-boolean-or-list safety knob in this codebase has one).

## 1. TOML config shape

```toml
[agent]
injection_scan_enabled = true
injection_scan_exempt = ["gmail.read", "calendar.list_events"]
```

- `injection_scan_enabled`: `Sourced<bool>` on `AivyxConfig`
  (`crates/aivyx-config/src/lib.rs`), matching `require_enforcement`'s
  exact shape (`pub require_enforcement: Sourced<bool>`, defaulting
  `true`, `FieldSource::Default` when absent). Fail-closed-by-default
  posture: the safety feature is on unless deliberately turned off,
  same as every other security-posture boolean in this config.
- `injection_scan_exempt`: plain `Vec<String>` (**not** wrapped in
  `Sourced` — a list's per-item provenance isn't a meaningful concept
  the way a single scalar's is), matching `allow_sensitive_paths:
  Vec<PathBuf>`'s exact precedent. Defaults to an empty vec when the
  key is absent. Entries are matched exactly against `Tool::name()`
  (e.g. `"gmail.read"`, `"web.fetch"`) — not validated against a
  known-tools registry at config-load time, matching `tool_allowlist`'s
  own existing behavior (a typo silently never matches at runtime
  rather than erroring at load time; this is accepted, not a gap this
  phase introduces).

## 2. Agent-level threading (`crates/aivyx-core/src/agent.rs`)

Two new fields on `ConcreteAgent`, matching the `tool_allowlist`/
`checkpointer` idiom exactly — plain fields, consuming-and-returning
builder methods, defaults that preserve current behavior byte-for-byte
if never explicitly set:

```rust
/// Chapter Picket Finding 3 follow-up — global on/off for the active
/// injection scan (`check_for_injection`). `true` (the default)
/// preserves Chapter Picket's original behavior byte-for-byte;
/// `false` disables the scan entirely while leaving Bulwark's fencing
/// untouched.
injection_scan_enabled: bool,
/// Chapter Picket Finding 3 follow-up — tool names exempted from the
/// active scan even when `injection_scan_enabled` is `true`. Matched
/// exactly against `Tool::name()`. Empty (the default) preserves
/// Chapter Picket's original behavior byte-for-byte.
injection_scan_exempt: std::collections::BTreeSet<String>,
```

`ConcreteAgent::new`'s field-initializer list gets
`injection_scan_enabled: true` and
`injection_scan_exempt: std::collections::BTreeSet::new()` added,
matching how `tool_allowlist: None` is initialized there today.

```rust
pub fn with_injection_scan_enabled(mut self, enabled: bool) -> Self {
    self.injection_scan_enabled = enabled;
    self
}

pub fn with_injection_scan_exempt(
    mut self,
    exempt: std::collections::BTreeSet<String>,
) -> Self {
    self.injection_scan_exempt = exempt;
    self
}
```

## 3. Integration point (`run_tool_call`, ~`agent.rs:1410`)

Find:

```rust
let mut injection_reason: Option<String> = None;
if tool.output_is_untrusted() {
    if let ToolOutcome::Completed { output, .. } = &mut outcome {
        injection_reason = check_for_injection(output, tool_name);
        let taken = std::mem::replace(output, serde_json::Value::Null);
        *output = fence_untrusted_output(taken, tool_name);
    }
}
```

Replace with:

```rust
let mut injection_reason: Option<String> = None;
if tool.output_is_untrusted() {
    if let ToolOutcome::Completed { output, .. } = &mut outcome {
        if self.injection_scan_enabled
            && !self.injection_scan_exempt.contains(tool_name)
        {
            injection_reason = check_for_injection(output, tool_name);
        }
        let taken = std::mem::replace(output, serde_json::Value::Null);
        *output = fence_untrusted_output(taken, tool_name);
    }
}
```

Only the `check_for_injection` call is gated. `fence_untrusted_output`
runs unconditionally immediately after, exactly as today — Bulwark's
protection is never affected by either knob.

## 4. Binary wiring (`crates/aivyx-cli/src/bin/aivyx.rs`)

Two real `ConcreteAgent::new(...)` construction sites exist in this
binary — `child_agent` (~line 8591, built inside the team/specialist
factory closure) and `daemon_agent` (~line 9087, the main daemon
agent). Both already thread `.with_tool_allowlist(...)` and
`.with_checkpointer(...)` from shared local variables unwrapped once
near where `require_enforcement`/`guard_sensitive_paths` are unwrapped
(~line 6382). The two new config values get unwrapped the same way and
threaded into **both** construction chains via
`.with_injection_scan_enabled(...)` / `.with_injection_scan_exempt(...)`
— child agents (team specialists) inherit the same operator-configured
posture as the daemon's own agent, consistent with how
checkpointing/tool-allowlisting already work identically for both.

No dedicated CLI subcommand. TOML-only, matching
`guard_sensitive_paths`/`allow_private_egress`'s precedent — no
existing plain-boolean-or-list safety knob in this codebase has a
dedicated `aivyx <name> set/show` subcommand; those exist only for
multi-valued, composable dials (`access`, `autonomy`).

## 5. Testing

- **`aivyx-config`**: a config-loading test confirming
  `injection_scan_enabled` defaults to `true` with
  `FieldSource::Default` when absent from TOML, and correctly parses
  an explicit `false` plus a populated `injection_scan_exempt` list
  when present — matching the existing test shape already covering
  `require_enforcement`.
- **`aivyx-core`**: three new unit tests on `run_tool_call`'s
  integration point, reusing the `FakeTool`/injection-marker fixtures
  already established by Chapter Picket's own test suite (the direct
  precedent: `injection_marker_in_untrusted_output_escalates_the_turn`
  in `agent.rs`):
  1. `injection_scan_disabled_globally_skips_the_scan_but_still_fences`
     — `with_injection_scan_enabled(false)`, injection-marker content,
     asserts the turn does **not** escalate but the tool's output is
     still Bulwark-fenced.
  2. `injection_scan_exempt_tool_skips_the_scan_but_still_fences` —
     `with_injection_scan_exempt({"web.fetch"})`, same two assertions
     for that specific tool.
  3. `injection_scan_still_fires_for_non_exempt_tools_when_others_are_exempt`
     — confirms the exemption is genuinely per-tool-name, not
     accidentally global, using two different tool names in one test
     (one exempted, one not, both carrying the same marker).

## Self-review

- **Placeholder scan:** none — every field name, TOML key, code block,
  and construction-site line number is given concretely, grounded
  against the real current file content.
- **Internal consistency:** Section 1's TOML shape, Section 2's agent
  fields, and Section 3's integration point all agree on field names
  (`injection_scan_enabled`, `injection_scan_exempt`) and types
  (`bool`, `BTreeSet<String>` at the agent layer / `Vec<String>` at
  the raw TOML layer — the `Vec` → `BTreeSet` conversion happens at
  the binary-wiring layer, Section 4, the same place `tool_allowlist`
  already does an analogous `Vec<String>` → `BTreeSet<String>`
  conversion).
- **Scope check:** one phase, three well-bounded layers (config, agent,
  binary wiring) plus tests — comparable in size to a single Chapter
  Picket phase, no decomposition needed.
- **Ambiguity check:** the "tune" half of the original Finding 3
  framing is resolved concretely as the exemption list (confirmed with
  the user during brainstorming, in preference to the simpler
  boolean-only option); audit visibility is resolved as "quiet,
  matching every existing precedent" (also confirmed with the user).

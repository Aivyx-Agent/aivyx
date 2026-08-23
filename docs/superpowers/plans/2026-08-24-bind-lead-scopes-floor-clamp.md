# `bind_lead_scopes` Floor-Clamp Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the deferred root-cause item from Piece A of the Team-Mission Triggers initiative: `bind_lead_scopes` currently unions a vertical pack's own declared lead-role capability scopes into the real daemon floor unconditionally; this plan clamps it to only pass through narrow orchestration markers, mirroring the specialist branch's own already-correct filtering.

**Architecture:** Extract a small, pure, directly-testable helper (`filter_lead_pack_scopes`) that separates a pack's own declared scopes into "kept" (orchestration markers) and "dropped" (everything else — out-of-floor domain scopes), then use it in `bind_lead_scopes`'s lead branch instead of the current unconditional union, logging a warning only when something real gets dropped.

**Tech Stack:** Rust, the existing `aivyx-channel` test conventions this file already establishes.

## Global Constraints

- The lead must still receive the **entire** real floor unconditionally — that property is correct and required (the lead needs full floor authority to grant onward to specialists) and must not change.
- Only `team.message`, `team.delegate`, and qualified `mcp.call:<server>:*` markers may flow through from a pack's own declared lead-role scopes; every other declared scope (any domain scope: `fs.*`, `shell.*`, `net.*`, etc.) must be dropped, regardless of what the pack itself claims.
- No files change other than `crates/aivyx-channel/src/team_mission_driver.rs`. `TeamConfig::load`'s own parsing is untouched.
- The two existing tests (`bind_lead_scopes_grants_per_role_and_stays_least_privilege`, `bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`) must pass **unmodified** — proving the fix is additive/narrowing, not a behavior change to the specialist branch or to the lead's legitimate floor-plus-markers behavior.
- A warning fires only when a real clamp happens (the pack declared something that got dropped) — never on every call, never when the pack's own declarations are already a subset of what's allowed through.

---

## File Structure

- **Modify** `crates/aivyx-channel/src/team_mission_driver.rs` — add `filter_lead_pack_scopes` (a new pure function, placed immediately above `bind_lead_scopes`, near the existing `scope_base` helper it also lives next to), and change `bind_lead_scopes`'s lead branch to use it.

---

### Task 1: Clamp the lead's pack-declared scopes to the floor

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs:1339-1405` (the `scope_base` helper and `bind_lead_scopes` function, plus their surrounding test module further down in the same file)

**Interfaces:**
- Produces: `fn filter_lead_pack_scopes(pack_declared: &[String]) -> (Vec<String>, Vec<String>)` (kept, dropped) — crate-private (`fn`, not `pub fn`), used only within this file.

- [ ] **Step 1: Write the failing tests for the new pure helper**

Add to `crates/aivyx-channel/src/team_mission_driver.rs`'s existing `mod tests` (the same module `bind_lead_scopes_grants_per_role_and_stays_least_privilege` lives in):

```rust
#[test]
fn filter_lead_pack_scopes_keeps_only_orchestration_markers() {
    let declared: Vec<String> = [
        "team.delegate",
        "team.message",
        "mcp.call:aviation-weather:*",
        "mcp.call", // bare marker — must NOT be kept (would grant every server)
        "shell.exec:cwd:/etc/**",
        "fs.write:/root/**",
        "net.fetch",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let (kept, dropped) = filter_lead_pack_scopes(&declared);

    assert!(kept.contains(&"team.delegate".to_string()));
    assert!(kept.contains(&"team.message".to_string()));
    assert!(kept.contains(&"mcp.call:aviation-weather:*".to_string()));
    assert_eq!(kept.len(), 3, "only the three real orchestration markers are kept, got: {kept:?}");

    assert!(dropped.contains(&"mcp.call".to_string()), "the bare marker is dropped, not kept");
    assert!(dropped.contains(&"shell.exec:cwd:/etc/**".to_string()));
    assert!(dropped.contains(&"fs.write:/root/**".to_string()));
    assert!(dropped.contains(&"net.fetch".to_string()));
    assert_eq!(dropped.len(), 4, "the four out-of-floor domain scopes are dropped, got: {dropped:?}");
}

#[test]
fn filter_lead_pack_scopes_drops_nothing_when_pack_declares_only_markers() {
    let declared: Vec<String> = ["team.delegate", "team.message"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (kept, dropped) = filter_lead_pack_scopes(&declared);
    assert_eq!(kept.len(), 2);
    assert!(dropped.is_empty(), "nothing to clamp when the pack only declares markers");
}

#[test]
fn filter_lead_pack_scopes_handles_empty_input() {
    let (kept, dropped) = filter_lead_pack_scopes(&[]);
    assert!(kept.is_empty());
    assert!(dropped.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib filter_lead_pack_scopes -- --test-threads=1`
Expected: FAIL to compile — `filter_lead_pack_scopes` doesn't exist yet.

- [ ] **Step 3: Write the failing test for `bind_lead_scopes`'s own clamping behavior**

Add to the same `mod tests`, immediately after the two existing `bind_lead_scopes_*` tests:

```rust
#[test]
fn bind_lead_scopes_clamps_pack_declared_domain_scopes_for_the_lead() {
    let floor: Vec<String> = [
        "memory.read",
        "memory.write",
        "fs.read:/root/**",
        "net.fetch",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut config = default_nonagon();
    // Simulate a careless or malicious third-party pack: the lead's OWN
    // declared capability_scopes claim a domain scope the real floor
    // never granted (fs.write, and shell.exec entirely).
    {
        let lead_name = config.lead.clone();
        let lead = config.members.iter_mut().find(|m| m.name == lead_name).unwrap();
        lead.capability_scopes.push("fs.write:/root/**".to_string());
        lead.capability_scopes.push("shell.exec:cwd:/root/**".to_string());
    }

    bind_lead_scopes(&mut config, &floor);

    let lead_name = config.lead.clone();
    let lead_caps = config
        .members
        .iter()
        .find(|m| m.name == lead_name)
        .unwrap()
        .capability_scopes
        .clone();

    // The floor's own scopes still flow through unconditionally.
    assert!(lead_caps.contains(&"net.fetch".to_string()));
    assert!(lead_caps.contains(&"fs.read:/root/**".to_string()));
    // The pack's own out-of-floor domain declarations do NOT survive —
    // this is the actual fix: previously these would have been unioned
    // in via the pack's own claim, regardless of the real floor.
    assert!(
        !lead_caps.contains(&"fs.write:/root/**".to_string()),
        "a pack-declared domain scope beyond the floor must be clamped"
    );
    assert!(
        !lead_caps.iter().any(|s| s.starts_with("shell.exec")),
        "a pack-declared shell scope, entirely absent from the floor, must be clamped"
    );
    // Legitimate orchestration markers the default roster already
    // declares for the lead still flow through unchanged.
    assert!(lead_caps.iter().any(|s| s == "team.delegate"));
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p aivyx-channel --lib bind_lead_scopes_clamps_pack_declared_domain_scopes_for_the_lead -- --test-threads=1`
Expected: FAIL — the assertion `!lead_caps.contains(&"fs.write:/root/**".to_string())` fails, since the current buggy code unions the pack's own declared `fs.write:/root/**` in unconditionally.

- [ ] **Step 5: Implement `filter_lead_pack_scopes`**

In `crates/aivyx-channel/src/team_mission_driver.rs`, immediately above `bind_lead_scopes` (after `scope_base`, ~line 1342), add:

```rust
/// Piece D (2026-08-24) — separates a pack's own declared capability_scopes
/// for the LEAD role into what's allowed through (`kept`: narrow
/// orchestration markers — team bus + qualified MCP grants, mirroring the
/// specialist branch's own already-correct filter below) and what must be
/// clamped (`dropped`: any domain scope — fs.*/shell.*/net.*/etc. — a pack
/// tried to add beyond the real daemon floor). Pure and side-effect-free
/// specifically so the "what got clamped" computation is directly testable
/// without needing to capture the warning log's own text.
///
/// This closes a real defense-in-depth gap: `TeamConfig::load` only
/// validates that a pack's own declared scopes *parse*, never that
/// they're authorized against the daemon's real floor — so an operator
/// installing an unaudited third-party vertical pack could otherwise have
/// its own declared lead scopes silently escalate beyond what the
/// operator's own `aivyx.toml`/trust-tier config actually grants.
fn filter_lead_pack_scopes(pack_declared: &[String]) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for s in pack_declared {
        let b = scope_base(s);
        if b == "team.message" || b == "team.delegate" || (b.starts_with("mcp.") && s.contains(':')) {
            kept.push(s.clone());
        } else {
            dropped.push(s.clone());
        }
    }
    (kept, dropped)
}
```

- [ ] **Step 6: Run the helper's own tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib filter_lead_pack_scopes -- --test-threads=1`
Expected: PASS, all 3 new tests.

- [ ] **Step 7: Wire the helper into `bind_lead_scopes`'s lead branch**

In `bind_lead_scopes` (currently ~line 1362-1370), replace:

```rust
        if m.name == lead_name {
            // The lead holds the full daemon authority (to grant), plus its own
            // orchestration scopes (team.delegate / team.message).
            let mut caps: Vec<String> = lead_scopes.to_vec();
            caps.extend(m.capability_scopes.iter().cloned());
            caps.sort();
            caps.dedup();
            m.capability_scopes = caps;
        } else {
```

with:

```rust
        if m.name == lead_name {
            // The lead holds the full daemon authority (to grant), plus only
            // narrow orchestration scopes (team.delegate / team.message /
            // qualified MCP) a pack's own capability_scopes declared for it
            // — never an arbitrary domain scope beyond the real floor. See
            // `filter_lead_pack_scopes`'s own doc comment for the full
            // defense-in-depth rationale.
            let mut caps: Vec<String> = lead_scopes.to_vec();
            let (kept, dropped) = filter_lead_pack_scopes(&m.capability_scopes);
            if !dropped.is_empty() {
                eprintln!(
                    "aivyx team: pack's own declared lead scopes exceed the daemon \
                     floor, clamped: {}",
                    dropped.join(", ")
                );
            }
            caps.extend(kept);
            caps.sort();
            caps.dedup();
            m.capability_scopes = caps;
        } else {
```

Leave the `else` branch (the specialist filtering logic) completely unchanged.

- [ ] **Step 8: Run all bind_lead_scopes-related tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib bind_lead_scopes -- --test-threads=1`
Expected: PASS, all 4 tests — the 2 pre-existing tests (`bind_lead_scopes_grants_per_role_and_stays_least_privilege`, `bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`) pass **unmodified**, proving no regression to the specialist branch or to the lead's own legitimate floor-plus-markers behavior; the new `bind_lead_scopes_clamps_pack_declared_domain_scopes_for_the_lead` test now passes.

- [ ] **Step 9: Run the crate's full suite**

Run: `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: PASS, no regressions (baseline before this task: 1295 tests — report the exact new count, expect baseline + 6: 3 `filter_lead_pack_scopes` tests + 1 `bind_lead_scopes` clamp test, plus confirm the 2 pre-existing `bind_lead_scopes` tests are unchanged in the diff).

- [ ] **Step 10: Confirm no unrelated diff**

Run: `git diff --stat` and confirm exactly one file changed: `crates/aivyx-channel/src/team_mission_driver.rs`. Confirm via `git diff` that the two pre-existing tests (`bind_lead_scopes_grants_per_role_and_stays_least_privilege`, `bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`) show **zero** lines changed — only additions elsewhere in the file.

- [ ] **Step 11: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "fix(team): clamp pack-declared lead scopes to the daemon floor

Closes the root-cause item Piece A's own Critical finding left
deferred: bind_lead_scopes previously unioned a vertical pack's own
declared lead-role capability_scopes into the real daemon floor
unconditionally, rather than intersecting against it. A third-party
pack could therefore declare domain scopes (fs.*/shell.*/net.*/etc.)
beyond what the operator's own trust-tier/aivyx.toml floor actually
grants, and bind_lead_scopes would silently admit them.

The concrete exploit path (schedule.create's model-facing pack_config
parameter) was already closed in Piece A's own final review; this is
a defense-in-depth fix for operators installing unaudited third-party
vertical packs, not an active exploit closure.

Mirrors the specialist branch's own already-correct filtering (only
team.message/team.delegate/qualified mcp.call:*:* markers flow through
from a pack's own declarations) via a new, directly-testable
filter_lead_pack_scopes helper. The lead still receives the whole real
floor unconditionally, unchanged -- only the pack's own additive
claims are now clamped. Logs a warning when a clamp actually removes
something. The two pre-existing bind_lead_scopes tests pass unmodified,
confirming no regression to intentional behavior."
```

---

## Final Verification

1. `cargo build -p aivyx-channel` — clean.
2. `cargo test -p aivyx-channel --lib -- --test-threads=1` — full suite passes, no regressions.
3. `cargo clippy -p aivyx-channel --all-targets -- -D warnings` — no new warnings beyond the one pre-existing, out-of-scope `trigger.rs:223` finding.
4. Manually trace the fix by reading (not running) the final diff: a third-party vertical pack whose lead role declares `shell.exec:cwd:/anywhere/**` — a scope the operator's own floor never granted — has that scope dropped by `filter_lead_pack_scopes` and never reaches the lead's final `capability_scopes`, with a warning logged naming exactly what was clamped. The lead still holds the operator's own real floor in full, unchanged.

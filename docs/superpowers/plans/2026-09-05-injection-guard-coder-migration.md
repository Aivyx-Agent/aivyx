# Phase 195 — Chapter Picket, Phase 2: Migrate aivyx-coder onto aivyx-injection-guard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `aivyx-coder`'s `aivyx-sandbox` crate stops maintaining its own copy
of the prompt-injection tripwire and depends on `aivyx-injection-guard`
instead — the same relationship it already has with `aivyx-confine` for
process confinement. Also correct the one real, stale doc claim found this
session in `aivyx-coder`'s own `CLAUDE.md`.

**Architecture:** Add `aivyx-injection-guard` as a pinned-rev git dependency
in `aivyx-coder`'s root `Cargo.toml` (mirroring the existing `aivyx-confine`/
`aivyx-checkpoint`/`aivyx-kvcache`/`aivyx-recall` entries exactly), then
change exactly two lines in `aivyx-sandbox/src/lib.rs` (drop the local
module, repoint the re-export at the external crate) and delete the now-
redundant local file. Every other call site in `aivyx-coder`
(`agent/mod.rs`, `delegate.rs`, `confirmation.rs`, `agent_builder.rs`,
`app.rs`, and their test modules) imports `InjectionFinding`/`InjectionTaint`/
`scan_for_injection_markers` via `aivyx_sandbox::{...}` — the crate-level
re-export, not the submodule path directly (confirmed via `grep -rln
"injection_scan::"` finding only `lib.rs` itself) — so none of them need any
change at all.

**Tech Stack:** Rust (2024 edition), `git`.

## Global Constraints

- Pin `aivyx-injection-guard` at rev `8d8583f9cd63dd48b72ffea9455f98f5a9405ffc`
  — the current, real `HEAD` of `Aivyx-Agent/aivyx-injection-guard` at the
  time this plan was written (confirmed via `git ls-remote`). Verify this is
  still the correct rev to pin before using it (re-run `git ls-remote
  https://github.com/Aivyx-Agent/aivyx-injection-guard` — if `HEAD` differs
  from this value, stop and report it rather than silently pinning a
  different commit than this plan was designed against).
- Public API names consumed from the new crate
  (`scan_for_injection_markers`, `InjectionFinding`, `InjectionTaint`) must
  not be renamed or wrapped — every existing call site in `aivyx-coder`
  depends on these exact names via the `aivyx_sandbox::{...}` re-export.
- Deleting `crates/aivyx-sandbox/src/injection_scan.rs` removes its 10
  `#[test]` functions from `aivyx-coder`'s own test count. This is
  expected, not a regression — those exact tests now live (byte-identical)
  in `aivyx-injection-guard`'s own repo, verified there in Phase 194. Do
  not attempt to keep a duplicate copy of them in `aivyx-coder`.
- `README.md`'s own "Known limitations" section (the real, current text,
  read in full this session) is **already accurate** — it correctly
  describes the scan-and-pause mechanism, its heuristic/pattern-based
  nature, and that it only runs in autonomous mode. **Do not edit
  `README.md`** — only `CLAUDE.md`'s condensed one-line summary is stale
  (see Task 2).
- Out of scope for this phase: adopting `aivyx-injection-guard` into
  `aivyx` itself (a separate, later phase).

---

### Task 1: Wire the dependency and migrate `aivyx-sandbox`'s module

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx-coder/Cargo.toml` (add to
  `[workspace.dependencies]`, after the existing pinned-rev block)
- Modify: `/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/
  Cargo.toml` (add the new dependency)
- Modify: `/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/
  src/lib.rs:1-26` (drop the local module, repoint the re-export, update
  the file-header doc comment)
- Delete: `/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/
  src/injection_scan.rs`
- Modify: `/home/julian/Projects/Rust/aivyx-coder/Cargo.lock` (regenerated,
  not hand-edited)

**Interfaces:**
- Consumes: `aivyx_injection_guard::{scan_for_injection_markers,
  InjectionFinding, InjectionTaint}` (the crate created in Phase 194).
- Produces: `aivyx_sandbox::{InjectionFinding, InjectionTaint,
  scan_for_injection_markers}` continues to resolve to the exact same
  names and signatures as before — every existing consumer in this repo is
  unaffected by this task.

- [ ] **Step 1: Add the pinned-rev dependency to the workspace root**

In `/home/julian/Projects/Rust/aivyx-coder/Cargo.toml`, find:

```toml
aivyx-confine = { git = "https://github.com/Aivyx-Agent/aivyx-confine", rev = "9a9ea5b96e6f97bc196722ebae4c46f61b4750a4", default-features = false }
aivyx-checkpoint = { git = "https://github.com/Aivyx-Agent/aivyx-checkpoint", rev = "37b0ebf200eaf5b31bcb795dfb1beb3356cd12a3" }
aivyx-kvcache = { git = "https://github.com/Aivyx-Agent/aivyx-kvcache", rev = "e1b06c9960ee98841d9b91978a11dd99ed388490" }
aivyx-recall = { git = "https://github.com/Aivyx-Agent/aivyx-recall", rev = "e22e28ac9dbcc051b9564c690d5082f79a51cf03" }
```

Add a new line immediately after the `aivyx-recall` line (still inside the
same centralized pinned-dependency block, same comment above it applies):

```toml
aivyx-injection-guard = { git = "https://github.com/Aivyx-Agent/aivyx-injection-guard", rev = "8d8583f9cd63dd48b72ffea9455f98f5a9405ffc" }
```

- [ ] **Step 2: Add the dependency to `aivyx-sandbox`**

In `/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/Cargo.toml`,
find:

```toml
[dependencies]
aivyx-confine = { workspace = true }
```

Change to:

```toml
[dependencies]
aivyx-confine = { workspace = true }
aivyx-injection-guard = { workspace = true }
```

- [ ] **Step 3: Repoint `lib.rs`'s module declaration and re-export**

In `/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/src/lib.rs`,
find:

```rust
//! Security boundary for tool execution.
//!
//! `PermissionGate` is the decision point every tool call must pass through
//! before `Tool::execute` runs; `ConfirmationGate` is the real (prompting)
//! implementation. `ExecutionConfiner` is the hook for OS-level process
//! confinement (Landlock + seccomp-bpf) — its real implementation,
//! `LandlockConfiner`, and `NoopConfiner` (its no-op fallback) both live
//! in the `aivyx-confine` crate now, re-exported here so every existing
//! call site in this workspace keeps working unchanged. See that crate's
//! own README for the confinement contract itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use aivyx_confine::{is_bare_pattern, is_basename_glob_match};

mod confirmation;
mod editor_approval;
mod injection_scan;
pub use aivyx_confine::{ExecutionConfiner, NoopConfiner, default_confiner};
#[cfg(feature = "sandbox-backend")]
pub use aivyx_confine::LandlockConfiner;
pub use confirmation::ConfirmationGate;
pub use injection_scan::{InjectionFinding, InjectionTaint, scan_for_injection_markers};
```

Replace with:

```rust
//! Security boundary for tool execution.
//!
//! `PermissionGate` is the decision point every tool call must pass through
//! before `Tool::execute` runs; `ConfirmationGate` is the real (prompting)
//! implementation. `ExecutionConfiner` is the hook for OS-level process
//! confinement (Landlock + seccomp-bpf) — its real implementation,
//! `LandlockConfiner`, and `NoopConfiner` (its no-op fallback) both live
//! in the `aivyx-confine` crate now, re-exported here so every existing
//! call site in this workspace keeps working unchanged. See that crate's
//! own README for the confinement contract itself. The prompt-injection
//! phrase-list tripwire (`scan_for_injection_markers`, `InjectionFinding`,
//! `InjectionTaint`) similarly now lives in the `aivyx-injection-guard`
//! crate, re-exported here the same way — extracted 2026-09-05 so `aivyx`
//! (the flagship Personal Assistant) can share the same primitive.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use aivyx_confine::{is_bare_pattern, is_basename_glob_match};

mod confirmation;
mod editor_approval;
pub use aivyx_confine::{ExecutionConfiner, NoopConfiner, default_confiner};
#[cfg(feature = "sandbox-backend")]
pub use aivyx_confine::LandlockConfiner;
pub use aivyx_injection_guard::{InjectionFinding, InjectionTaint, scan_for_injection_markers};
pub use confirmation::ConfirmationGate;
```

(Note: `pub use confirmation::ConfirmationGate;` moved below the new
`aivyx_injection_guard` re-export purely to keep the two external-crate
re-exports grouped together, alphabetically, matching this file's existing
convention of grouping `aivyx_confine` re-exports together above it — not a
functional change.)

- [ ] **Step 4: Delete the now-redundant local module**

```bash
rm /home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/src/injection_scan.rs
```

- [ ] **Step 5: Regenerate `Cargo.lock` and confirm the dependency resolves**

```bash
cd /home/julian/Projects/Rust/aivyx-coder
cargo check --workspace 2>&1 | tail -20
```

Expected: clean compile, no errors. This is the step that would surface a
typo in the rev, a genuine API mismatch, or a missed call site — if it
fails, do not proceed; diagnose and fix before continuing.

- [ ] **Step 6: Run the full workspace test suite and clippy**

```bash
cd /home/julian/Projects/Rust/aivyx-coder
cargo test --workspace 2>&1 | tail -40
cargo clippy --workspace --all-targets 2>&1 | tail -20
```

Expected: all tests pass (the total count will be 10 lower than before
this task — see Global Constraints — this is expected, not a failure to
investigate), and clippy reports no warnings. Every test that exercises
`InjectionTaint`/`scan_for_injection_markers` indirectly (in
`confirmation.rs`, `agent/mod.rs`, `agent/tests.rs`, `delegate.rs`) should
still pass unchanged, since they call the same re-exported names.

- [ ] **Step 7: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx-coder
git add -A
git commit -m "Depend on aivyx-injection-guard instead of the local injection_scan module

Same relationship aivyx-sandbox already has with aivyx-confine for
process confinement. Every existing call site (agent/mod.rs,
delegate.rs, confirmation.rs, agent_builder.rs, app.rs, and their
tests) imports via the aivyx_sandbox::{...} re-export, not the
submodule path directly, so none of them needed any change --
confirmed via grep before making this change, not assumed.

The 10 tests that lived in the deleted injection_scan.rs now live,
byte-identical, in aivyx-injection-guard's own repo (verified there
when that repo was created) -- this repo's total test count drops
by exactly that many, which is expected, not a regression."
```

---

### Task 2: Correct the one stale doc claim in `CLAUDE.md`

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx-coder/CLAUDE.md`

**Interfaces:** None — documentation-only change, no code interfaces
affected.

`README.md`'s own "Known limitations" section already accurately describes
the real mechanism (confirmed by reading it in full this session: it
explains the scan-and-pause guard, that it's heuristic/pattern-based, that
it only runs in autonomous mode, and points to the taint-carries-across-
turns-and-sub-agents behavior). `CLAUDE.md`'s "Known, deliberately-
undefended limitations" section instead gives a condensed one-line summary
that says content "re-enters context untagged" — which is no longer
accurate on its own (content *is* actively scanned and, on a match, the
whole run *is* tainted) even though it's meant only as a pointer back to
README's fuller, correct treatment.

- [ ] **Step 1: Confirm the current exact wording**

```bash
grep -n "indirect prompt injection" /home/julian/Projects/Rust/aivyx-coder/CLAUDE.md
```

Expected: one match, in the "Known, deliberately-undefended limitations"
section, reading (verify this matches before editing — if the wording has
changed since this plan was written, adapt the find/replace below to the
real current text rather than blindly applying it):

```
Documented in `README.md` "Known limitations" — worth checking before
assuming a gap is a bug: indirect prompt injection (file/command content
re-enters context untagged), network is unrestricted for approved
commands, env vars are inherited by spawned commands, TOCTOU windows on
path resolution, and `git_commit` (re)stages full paths rather than
partial hunks.
```

- [ ] **Step 2: Fix the parenthetical**

Find:

```markdown
Documented in `README.md` "Known limitations" — worth checking before
assuming a gap is a bug: indirect prompt injection (file/command content
re-enters context untagged), network is unrestricted for approved
commands, env vars are inherited by spawned commands, TOCTOU windows on
path resolution, and `git_commit` (re)stages full paths rather than
partial hunks.
```

Replace with:

```markdown
Documented in `README.md` "Known limitations" — worth checking before
assuming a gap is a bug: indirect prompt injection (a heuristic
scan-and-pause guard exists in autonomous mode — see `aivyx-sandbox`'s
`InjectionTaint`/`scan_for_injection_markers`, now sourced from the
`aivyx-injection-guard` crate — but it's pattern-based, not structural,
and doesn't run in interactive mode at all), network is unrestricted for
approved commands, env vars are inherited by spawned commands, TOCTOU
windows on path resolution, and `git_commit` (re)stages full paths rather
than partial hunks.
```

- [ ] **Step 3: Verify no other stale mentions exist**

```bash
grep -rn "re-enters context untagged\|re-enters the model's context with no" /home/julian/Projects/Rust/aivyx-coder/*.md /home/julian/Projects/Rust/aivyx-coder/crates/*/README.md 2>/dev/null
```

Expected: no remaining matches of the specific "untagged"/"with no
trust/provenance tag" phrasing that implies zero detection — `README.md`'s
own occurrence (in its `## Known limitations` section) already says
"re-enters the model's context with no trust/provenance tag" as a true,
narrower claim (the model itself can't structurally tell user input from
file content apart, which remains true even with the heuristic scan
running) — this is accurate and must NOT be changed. Only `CLAUDE.md`'s
sentence, which implied no detection at all, was wrong.

- [ ] **Step 4: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx-coder
git add CLAUDE.md
git commit -m "docs: correct CLAUDE.md's stale 'injection re-enters context untagged' claim

README.md's own Known Limitations section already accurately
describes the real scan-and-pause mechanism (autonomous mode only,
heuristic, tags the whole run via InjectionTaint on a match) --
CLAUDE.md's condensed one-line summary said content re-enters
context untagged, which is no longer true on its own. Also updates
the reference to note the scan now lives in the aivyx-injection-guard
crate, not this repo's own local module (see the sibling migration
commit)."
```

---

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** the design spec's "aivyx-coder migrates its own call
  sites onto this crate and removes its local copy" is Task 1; the design
  spec's "Declined alongside this survey" section already established the
  doc-correction was found stale — Task 2 fixes exactly that, and only
  that (confirmed `README.md` needs no change, narrowing what was
  originally assumed to be a two-file fix down to one).
- **No placeholders:** every step has literal, complete before/after code
  blocks or exact commands; the exact current `lib.rs`/`Cargo.toml`
  content was read from the real files before this plan was written, not
  guessed.
- **Type/interface consistency:** Task 1's re-exported names
  (`scan_for_injection_markers`, `InjectionFinding`, `InjectionTaint`)
  match Phase 194's plan and the real `aivyx-injection-guard` crate
  exactly — no drift introduced.

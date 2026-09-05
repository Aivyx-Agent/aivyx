# Phase 194 — Chapter Picket, Phase 1: Create the aivyx-injection-guard Repo — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `aivyx-coder`'s `injection_scan.rs` (the real, already-wired
prompt-injection phrase-list tripwire) into a new, standalone, public sibling
repo — `aivyx-injection-guard` — so both `aivyx-coder` and `aivyx` can share
one implementation, matching the established `aivyx-confine`/
`aivyx-checkpoint`/`aivyx-kvcache` extraction pattern exactly.

**Architecture:** A new, single-crate (no workspace) Rust repo scaffolded
identically to `aivyx-confine`: `Cargo.toml` (no dependencies — the ported
code is pure `std`), `src/lib.rs` (the full ported content, byte-identical
below its top doc comment), `README.md`, `CLAUDE.md`, `LICENSE-MIT` +
`LICENSE-APACHE`, `.gitignore`. Verified locally, then pushed to a real,
public `Aivyx-Agent/aivyx-injection-guard` GitHub repo.

**Tech Stack:** Rust (2024 edition, matching the sibling repos), `git`, `gh`.

## Global Constraints

- The ported code (`INJECTION_MARKERS`, `SCAN_WINDOW_BYTES`,
  `EXCERPT_CONTEXT_BYTES`, `InjectionFinding`, `InjectionTaint`,
  `floor_char_boundary`, `ceil_char_boundary`, `excerpt_around`,
  `scan_for_injection_markers`, and the full `#[cfg(test)] mod tests`
  block) must be **byte-identical** to
  `/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/src/
  injection_scan.rs` lines 14–260 — only the top-of-file doc comment
  (lines 1–12) changes, to describe a standalone crate rather than one
  consumer's specific input list. This is a verbatim extraction, not a
  rewrite — `aivyx-coder`'s own migration (a later, separate phase) needs
  its existing behavior and tests unchanged.
- License: `MIT OR Apache-2.0`, matching `aivyx-confine`/`aivyx-checkpoint`/
  `aivyx-kvcache`. Copy `LICENSE-MIT` and `LICENSE-APACHE` verbatim from
  `/home/julian/Projects/Rust/aivyx-confine/` (223 lines total of standard
  boilerplate — do not retype).
- No dependencies of any kind — the ported code only uses `std::sync::{Arc,
  Mutex}`. Do not add `tokio`, `serde`, or anything else "for later"; keep
  this crate as dependency-free as the code it's extracting.
- The new repo must end up **public** on GitHub (not private) — Phase 192
  (Chapter N) found and fixed exactly this mistake for the other three
  sibling repos; don't repeat it here. Verify with `gh repo view
  Aivyx-Agent/aivyx-injection-guard --json isPrivate` before considering
  this phase done.
- Out of scope for this phase: migrating `aivyx-coder` onto the new crate
  (a later phase), adopting it into `aivyx` (a later phase after that),
  and correcting `aivyx-coder`'s own stale README/CLAUDE.md line ("indirect
  prompt injection (file/command content re-enters context untagged)" —
  confirmed stale this session: the scanner is real, live, and wired into
  five real call sites) — that correction belongs to the migration phase,
  since it requires editing `aivyx-coder`'s own files.

---

### Task 1: Scaffold, populate, and verify the new crate locally

**Files:**
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/Cargo.toml`
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/src/lib.rs`
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/README.md`
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/CLAUDE.md`
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/.gitignore`
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/LICENSE-MIT`
  (copied verbatim from `aivyx-confine`)
- Create: `/home/julian/Projects/Rust/aivyx-injection-guard/LICENSE-APACHE`
  (copied verbatim from `aivyx-confine`)

**Interfaces:**
- Produces: a real, standalone git repo at
  `/home/julian/Projects/Rust/aivyx-injection-guard/` with a clean initial
  commit, ready for Task 2 to push to GitHub. Public API:
  `pub fn scan_for_injection_markers(text: &str, source: &str) ->
  Option<InjectionFinding>`, `pub struct InjectionFinding { pub source:
  String, pub matched_pattern: String, pub excerpt: String }`, `pub struct
  InjectionTaint` with `new()`/`flag(&self, finding: InjectionFinding)`/
  `current(&self) -> Option<InjectionFinding>`/`take(&self) ->
  Option<InjectionFinding>`. Later phases (the `aivyx-coder` migration, the
  `aivyx` adoption) depend on these exact names and signatures — do not
  rename anything during this extraction.

- [ ] **Step 1: Create the directory and initialize git**

```bash
mkdir -p /home/julian/Projects/Rust/aivyx-injection-guard/src
cd /home/julian/Projects/Rust/aivyx-injection-guard
git init
```

- [ ] **Step 2: Write `Cargo.toml`**

```toml
[package]
name = "aivyx-injection-guard"
description = "Heuristic prompt-injection phrase-list tripwire for untrusted agent input"
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"

[dependencies]
```

- [ ] **Step 3: Write `.gitignore`**

```
/target
```

- [ ] **Step 4: Copy the license files verbatim**

```bash
cp /home/julian/Projects/Rust/aivyx-confine/LICENSE-MIT /home/julian/Projects/Rust/aivyx-injection-guard/LICENSE-MIT
cp /home/julian/Projects/Rust/aivyx-confine/LICENSE-APACHE /home/julian/Projects/Rust/aivyx-injection-guard/LICENSE-APACHE
```

Verify both copied completely: `wc -l /home/julian/Projects/Rust/aivyx-injection-guard/LICENSE-MIT /home/julian/Projects/Rust/aivyx-injection-guard/LICENSE-APACHE` — expected `21` and `202` lines respectively (matching the source files).

- [ ] **Step 5: Write `src/lib.rs`**

The new top-of-file doc comment (replacing the original's consumer-specific
framing and now-wrong doc-path reference), followed by the **exact,
unmodified** body of `/home/julian/Projects/Rust/aivyx-coder/crates/
aivyx-sandbox/src/injection_scan.rs` from its line 14 (`use std::sync::
{Arc, Mutex};`) through its final line 260 (the closing `}` of the `tests`
module) — read that file and copy that byte range verbatim after the new
header below. Do not alter any identifier, constant value, doc comment
below the file header, or test:

```rust
//! Heuristic detection of likely prompt-injection markers in content that
//! enters an agent's context from outside the user's own direct input —
//! tool output, fetched pages, file contents, or any other untrusted
//! external content a consumer chooses to scan.
//!
//! This is a tripwire, not a classifier: a static phrase list will both
//! miss real injection attempts phrased differently and flag benign text
//! that happens to mention one of these phrases (including, ironically,
//! this crate's own docs/tests about this feature). That's an accepted
//! cost — the intended response to a match is "surface it for a human (or
//! an unattended-run policy) to judge," not a silent classifier verdict.
//!
//! Extracted from `aivyx-coder`'s own `aivyx-sandbox` crate (originally
//! `injection_scan.rs`) so `aivyx` (the flagship Personal Assistant) can
//! share the same primitive rather than reimplementing it — same
//! rationale as `aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`.
```

(Then the verbatim body — `use std::sync::{Arc, Mutex};` onward — copied
from the source file. If you cannot copy a byte range directly, re-read
`/home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/src/
injection_scan.rs` in full and transcribe lines 14–260 exactly, checking
afterward with `diff <(tail -n +14 /home/julian/Projects/Rust/aivyx-coder/
crates/aivyx-sandbox/src/injection_scan.rs) <(tail -n +14
/home/julian/Projects/Rust/aivyx-injection-guard/src/lib.rs)` — expected:
no output, meaning the two are identical from that point on.)

- [ ] **Step 6: Verify the diff is empty**

```bash
diff <(tail -n +14 /home/julian/Projects/Rust/aivyx-coder/crates/aivyx-sandbox/src/injection_scan.rs) <(tail -n +14 /home/julian/Projects/Rust/aivyx-injection-guard/src/lib.rs)
```

Expected: no output (exit code 0). If there's any diff, fix `src/lib.rs`
until this is empty — the Global Constraints section requires byte-identical
code below the header.

- [ ] **Step 7: Write `README.md`**

```markdown
# aivyx-injection-guard

Heuristic detection of likely prompt-injection markers in untrusted content
that enters an agent's context from outside the user's own direct input.

A case-insensitive phrase-list scan (`scan_for_injection_markers`) against a
fixed, deliberately-non-exhaustive set of known injection phrasings
("ignore previous instructions", "you are now", etc.), bounded to a 64KB
scan window, returning a bounded excerpt around any match. Explicitly a
tripwire, not a classifier — the intended response to a match is "surface it
for a human, or an unattended-run policy, to judge," never a silent
classifier verdict. `InjectionTaint` is a small `Arc<Mutex<Option<...>>>`
first-finding-wins shared flag a consumer can use to persist a match across
a session without inventing its own synchronization.

No dependencies — pure `std`.

Extracted 2026-09-05 from `aivyx-coder`'s own `aivyx-sandbox` crate
(originally `injection_scan.rs`), which now depends on this crate instead of
maintaining its own copy — same rationale, and the same pattern, as
`aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`.

See `docs/superpowers/specs/2026-09-05-injection-guard-design.md` in the
`aivyx` repo for the full design rationale.
```

- [ ] **Step 8: Write `CLAUDE.md`**

```markdown
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working
with code in this repository.

## What this is

`aivyx-injection-guard` is a small, dependency-free prompt-injection
phrase-list tripwire: `scan_for_injection_markers(text, source)` plus a
shared `InjectionTaint` flag. It exists so `aivyx-coder` and `aivyx` (the
flagship Personal Assistant) can share one implementation of the same
detection primitive, rather than each maintaining — and potentially
drifting on — its own copy. See `README.md` and `aivyx/docs/superpowers/
specs/2026-09-05-injection-guard-design.md` for the full rationale — this
file only covers what's specific to working in this repo's code.

## Build, test, lint

\`\`\`sh
cargo build
cargo test
cargo clippy --all-targets
cargo fmt
\`\`\`

Single crate, no workspace — no `-p` flag needed. Single test:
`cargo test <test_name>`.

## Architecture

Everything lives in `src/lib.rs` — there's no submodule split (unlike
`aivyx-confine`'s trait-vs-backend split), since this crate has no
alternate-backend concept: one scan function, one marker list, one shared
taint type.

- `INJECTION_MARKERS` — the fixed, deliberately-non-exhaustive phrase list.
  Every entry must be lowercase ASCII (the scan lowercases with
  `to_ascii_lowercase()`, not `to_lowercase()`, to keep byte offsets stable
  across full Unicode case-folding edge cases — see
  `scan_keeps_correct_byte_offsets_when_lowercasing_changes_length`'s test
  for why that distinction matters).
- `scan_for_injection_markers` — case-insensitive substring match within a
  bounded `SCAN_WINDOW_BYTES` (64KB) window, returning the first match by
  position *in the marker list*, not by position in the text.
- `InjectionFinding` — what tripped it (`matched_pattern`), where from
  (`source`, a caller-supplied human-readable label), and a bounded excerpt
  for a human to judge at a glance.
- `InjectionTaint` — an `Arc<Mutex<Option<InjectionFinding>>>` wrapper,
  first-finding-wins. Not required to use this crate's detection — a
  consumer can call `scan_for_injection_markers` directly and build its own
  response, the way `aivyx` does (converting a match directly into an
  existing turn-outcome type rather than persisting a taint flag).

## Where to look next

- `README.md` — quick orientation and the design-doc pointer.
- `aivyx/docs/superpowers/specs/2026-09-05-injection-guard-design.md` — the
  full design: why this was extracted, and how each of the two consumers
  (`aivyx-coder`, `aivyx`) integrates it differently.
```

- [ ] **Step 9: Build, test, and lint**

```bash
cd /home/julian/Projects/Rust/aivyx-injection-guard
cargo build 2>&1 | tail -10
cargo test 2>&1 | tail -20
cargo clippy --all-targets -- -D warnings 2>&1 | tail -15
```

Expected: `cargo build` and `cargo clippy` both finish clean with no
warnings; `cargo test` reports `test result: ok. 10 passed; 0 failed` (the
ported file has 10 `#[test]` functions — count them yourself in the source
if this exact number seems off, don't just trust this plan's count blindly).

- [ ] **Step 10: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx-injection-guard
git add -A
git commit -m "Extract injection_scan.rs from aivyx-coder's aivyx-sandbox crate

Verbatim port (byte-identical below the file header) of the phrase-list
prompt-injection tripwire aivyx-coder already uses in production,
wired into 5 real call sites (editor context, repo map, user/project
AGENTS.md, generic tool output) via ConfirmationGate and the
autonomous loop. Extracted so aivyx (the flagship Personal Assistant)
can share the same primitive instead of reimplementing it -- same
pattern as aivyx-confine/aivyx-checkpoint/aivyx-kvcache.

See aivyx's docs/superpowers/specs/2026-09-05-injection-guard-design.md
for the full design rationale."
```

---

### Task 2: Create the real, public GitHub repo and push

**Files:** None — this task creates a real, externally-visible GitHub
repository; no `aivyx` repo-local files change.

**Interfaces:**
- Consumes: the local git repo and its initial commit from Task 1.
- Produces: `https://github.com/Aivyx-Agent/aivyx-injection-guard`, public,
  containing exactly Task 1's commit — the pinned-rev git dependency later
  phases (the `aivyx-coder` migration, the `aivyx` adoption) will point at.

**Execution note — read before running this task:** creating a new,
public GitHub repository under the `Aivyx-Agent` organization is a real,
externally-visible action. Per the precedent from Phase 192 (repo
visibility) and Phase 193 (tag pushes) in Chapter N, this should **not**
be delegated to an autonomous subagent — the controller running this plan
should execute it directly and get the user's explicit go-ahead
immediately before running `gh repo create`.

- [ ] **Step 1: Confirm the repo name isn't already taken**

```bash
gh repo view Aivyx-Agent/aivyx-injection-guard 2>&1
```

Expected: an error indicating the repo doesn't exist (e.g. "Could not
resolve to a Repository" / `HTTP 404`). If it already exists, stop and
report this to the user — do not silently overwrite or reuse an existing
repo of that name.

- [ ] **Step 2: Get explicit go-ahead, then create the repo and push**

After confirming with the user this specific action should proceed now,
run (from inside the new local repo):

```bash
cd /home/julian/Projects/Rust/aivyx-injection-guard
gh repo create Aivyx-Agent/aivyx-injection-guard --public \
  --description "Heuristic prompt-injection phrase-list tripwire for untrusted agent input" \
  --source=. --remote=origin --push
```

- [ ] **Step 3: Verify the repo is real, public, and contains the real content**

```bash
gh repo view Aivyx-Agent/aivyx-injection-guard --json isPrivate,description --jq '{isPrivate, description}'
```

Expected: `{"isPrivate": false, "description": "Heuristic prompt-injection phrase-list tripwire for untrusted agent input"}`.

```bash
git ls-remote --tags origin 2>&1; git -C /home/julian/Projects/Rust/aivyx-injection-guard log --oneline
```

The second command's output (real commit log) should show exactly the one
commit from Task 1.

- [ ] **Step 4: Verify anonymous cloneability — the same real proof standard Phase 192 established**

```bash
GIT_TERMINAL_PROMPT=0 git -c credential.helper= ls-remote https://github.com/Aivyx-Agent/aivyx-injection-guard > /dev/null 2>&1 && echo "OK: anonymously cloneable" || echo "FAIL: not anonymously cloneable"
```

Expected: `OK: anonymously cloneable`. If this fails, the repo was likely
created with an unexpected default visibility — re-check Step 3's
`isPrivate` result and fix before considering this phase done. (Phase 194's
own successor — the future CI regression-guard phase from Chapter N — will
eventually also cover this repo the same way it covers
`aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`, once `aivyx` actually
depends on it; that wiring isn't part of this phase.)

There is no commit for this task — it creates external GitHub state, not
repo-local files. Record the outcome (repo created, public, content
verified, anonymous-clone-verified) in the task report for the reviewer.

---

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** the design spec's "aivyx-injection-guard (new sibling
  repo)" section is fully covered by this plan — Task 1 scaffolds and
  verifies the crate locally; Task 2 makes it real and public on GitHub.
  The spec's "Integration point in aivyx-core" and the `aivyx-coder`
  migration are explicitly out of scope here (later phases).
- **No placeholders:** every step has literal, complete file content or
  exact, runnable commands — the license files are copied (not retyped) to
  avoid transcription errors in 223 lines of legal boilerplate; the ported
  Rust code is verified byte-identical via `diff`, not just "should match."
- **Type/interface consistency:** the public API names (`scan_for_
  injection_markers`, `InjectionFinding`, `InjectionTaint`, `INJECTION_
  MARKERS`) are preserved exactly from the source file — later phases that
  depend on these names will not need to guess or reconcile a rename.

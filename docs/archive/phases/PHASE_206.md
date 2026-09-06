# Phase 206 — Configurable KV-Cache Store Path Sharing (Aivyx ↔ Aivyx Coder)

**Cross-repo interoperability audit follow-up — [SHIPPED] 2026-09-07.**

## Goal

Following a direct-code audit of whether `aivyx` and `aivyx-coder` can
interact (finding: yes, via a real, working, tested MCP bridge —
`aivyx-coder --mcp-server` exposing `code`/`code_reply` tools that
`aivyx`'s Nonagon team system bridges in as an out-of-process specialist),
a further audit for refinement opportunities found that `aivyx-kvcache`'s
`LlamaServerSlotStore` — built specifically so `aivyx` and `aivyx-coder`
could share expensive LLM prefill work across their otherwise-separate
`llama-server` processes — could never actually be shared in practice:
both consumers hardcoded their own app-name-scoped store path
(`~/.local/share/aivyx/kvcache` and `~/.local/share/aivyx-coder/kvcache`
respectively), with no config override on either side. This phase added
one to each.

## What shipped

- **Grounded the load-bearing safety question before designing anything**:
  read `aivyx-kvcache`'s own manifest source directly and found it
  already uses WAL-mode sqlite specifically because — per its own code
  comment — "aivyx-coder and aivyx each run their own llama-server-backed
  process today... so cross-process safety is a real requirement here."
  The crate was engineered for exactly this scenario from the start; no
  new concurrency-safety code was needed anywhere in this phase.
- **`aivyx-coder`**: `BackendSettings::kvcache_store_path` (tilde-expanded,
  `None` preserves the historical default) plus
  `resolved_kvcache_store_path()` as the single source of truth, reused
  by both the real kvcache-construction call site and a new
  `Settings::effective_deny_paths()` — which also fixed a real,
  independently-found gap folded into this same phase (the user's
  explicit choice, over logging it separately): the kvcache directory's
  `deny_paths` protection was a static string in
  `PermissionSettings::default()`, blind to any override.
- **`aivyx`**: `AivyxConfig::kvcache_store_path` (`[kvcache] store_path`
  in TOML, `AIVYX_KVCACHE_STORE_PATH` env override) plus
  `effective_kvcache_store_path()`, reused by both the kvcache
  construction and a new Ward `SensitivePolicy` extra-deny entry — `aivyx`
  had *no* existing protection at all for its own kvcache directory (Ward
  classifies by directory name or extension, neither of which matches
  `.slot`/manifest files), unlike `aivyx-coder`'s side, which already had
  one.
- **The final whole-branch review (Opus, covering both repos together)
  found the largest finding set of any phase this run: 1 Critical + 6
  Important + 3 Minor** — all real, all independently re-verified by the
  controller before fixing, none manufactured:
  - **Critical**: `docs/MCP_RECIPES.md`'s pairing subsection told
    operators to write a bare top-level `kvcache_store_path` key for
    `aivyx`, but the real loader reads a `[kvcache] store_path` section —
    an operator following the doc exactly would silently misconfigure
    `aivyx`'s side while `aivyx-coder`'s side worked, defeating the whole
    point without any error.
  - **Important**: the doc's core claim was itself wrong — `aivyx` and
    `aivyx-coder` compute their cache keys via structurally different
    hashing (confirmed by reading both `compute_prefix_hash`
    implementations directly), so the two processes can never actually
    produce a matching cache key. Sharing the directory does not mean
    either app reuses the other's prefill work. The real, corrected
    reason to share it: a single `llama-server` has exactly one
    `--slot-save-path`, so both configs must agree on the directory for
    save/restore size-accounting to work correctly at all once two apps
    point at one server — a correctness requirement, not an optional
    optimization.
  - **Important**: both repos' install docs (`aivyx`'s `INSTALL.md`,
    `aivyx-coder`'s `README.md`) still described the old hardcoded-only
    path with no mention of the new override.
  - **Important**: `aivyx`'s `effective_kvcache_store_path()` returned
    its result un-canonicalized, unlike the Ward allow-list right beside
    it in the same file — meaning the brand-new Ward protection this
    phase added could silently no-op behind a symlinked ancestor
    directory (common under dotfile managers). Fixed to canonicalize
    with the same exists-or-fallback pattern already used nearby.
  - **Important**: `aivyx-coder`'s `README.md` told operators to hand-add
    the kvcache path to `deny_paths` on existing installs — now actively
    wrong twice over, since protection is automatic on every install now,
    and hand-adding the *old* literal after configuring an override would
    protect the wrong path.
  - Also flagged and fixed: an unnecessary whole-`AivyxConfig` clone
    (secrets included) introduced to work around a destructure-ordering
    constraint, simplified to cloning only the resulting `PathBuf`; an
    undocumented shared-and-asymmetric eviction-budget interaction once
    two apps' `kvcache_max_bytes` settings both apply to one directory
    (documented as a caveat, not code-fixed — both default to 10 GiB, so
    it's invisible until an operator tunes one down).
- **A real infrastructure quirk was discovered and worked around
  mid-execution**: switching this session's own active worktree while a
  background subagent was still running caused one commit to land on the
  wrong (but harmless, same-repo, non-conflicting) branch — caught only
  by checking `git worktree list` directly rather than trusting the
  subagent's own reported commit hash, and worked around by continuing on
  the branch the commit actually landed on rather than force-fixing
  history. Lesson applied for the rest of the phase: no further worktree
  switches while any subagent was still active.
- **A separate, repeated agent-reliability issue**: `aivyx-coder`'s Task 1
  implementer needed three resumes because it kept ending its turn while
  a backgrounded `cargo test`/`clippy` command was still running, rather
  than waiting for it — each time diagnosed by checking real git/file
  state directly (not the "completed" notification's own text, which on
  the first two occasions was a self-authored progress note, not an
  actual completion), resolved by explicitly instructing the agent to run
  commands in the foreground and read real output before ending its turn.
- Every fix independently re-verified by the controller directly against
  real files and fresh test runs, not from subagent reports: both repos'
  full suites re-run after every commit including the final fixes —
  `aivyx-coder`: 23/23 test-result blocks, clippy clean; `aivyx`: 120/120
  test-result blocks, clippy clean (one transient failure during merged-
  main verification, `team_mission_driver::tests::pause_mission_succeeds_while_executing`,
  confirmed a pre-existing timing flake in a module this phase never
  touched — passed cleanly in isolation, and a full-suite re-run came
  back 120/120).

## The result

An operator can now point `aivyx` and a locally delegated
`aivyx-coder --mcp-server` process at the same directory to get correct
KV-cache slot-size accounting when both share one `llama-server` — the
capability `aivyx-kvcache` was built for in the first place, now actually
reachable from both consumers. `aivyx`'s own kvcache directory has Ward
protection for the first time. The pairing documentation states plainly
what sharing does and doesn't buy an operator, rather than the
overstated original framing.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx-kvcache`'s manifest has no `busy_timeout` set on its WAL-mode
  sqlite connection.** True concurrent two-process writes only become a
  live possibility because of this phase; `SQLITE_BUSY` on a `record`/
  `evict_and_remove` race is now reachable in practice, though not
  observed. Flagged by the final review as a `aivyx-kvcache`-side
  follow-up, deliberately not changed on this branch (out of scope —
  this phase touched only `aivyx` and `aivyx-coder`).
- **The shared eviction-budget asymmetry is documented, not enforced.**
  If an operator tunes `aivyx-coder`'s `kvcache_max_bytes` down while
  sharing a directory with `aivyx`, `aivyx-coder`'s smaller budget will
  evict `aivyx`'s slot files too. No code prevents this; the pairing doc
  now says so.
- **Chapter I's "polish" placeholder remains paused** (from Phase 205),
  and **`docs/POLISH_WAVES.md`'s possible naming collision with it**
  remains unexplored — neither touched by this phase.

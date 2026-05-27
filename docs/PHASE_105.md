# Phase 105 — Trajectory Logging (`aivyx audit export`) (Chapter D opener)

Chapter D opens here. The post-Phase-104 Hermes-comparison
pass named five out-of-the-box-surface gaps; Phase 105 is the
**easiest-wins-first** opener — a read-only emitter over the
existing HMAC audit chain that closes the trajectory-logging
gap without touching any locked decision.

The Aivyx audit chain has carried structured per-turn and
per-tool-call rows since Phase 6. Every `ToolCall` records
`(turn_id, tool_id, scope_used, input_hash, outcome,
duration)`; every `TurnStarted` records `(turn_id, session_id,
channel, trust_tier, effective_capabilities)`; `TurnEnded`
carries the outcome plus `tool_calls_made` and `usage`. The
data a research-grade trajectory exporter needs is **already
in the chain**, structured, signed, and serde-`Serialize`. No
new substrate is needed — only a reader.

Phase 105 ships `aivyx audit export`: an offline subcommand
that opens encrypted storage with the operator's passphrase
(same cold-start path as `aivyx --verify-only`), reads
`KeyDomain::Audit` via the existing
`PersistentAuditLog::entries_range`, and emits each
`SignedEntry` as one JSON line on stdout. `--from <seq>` and
`--limit <N>` map directly to the underlying reader. No
daemon needed; no IPC variant; no chain mutation. Pipe to a
file or to `jq`. The export is **complete enough to
re-verify the chain downstream** — every line carries `seq +
appended_at_ms + prev_mac + mac + event`, so a third-party
tool can replay the HMAC against a separately-supplied
genesis seed.

## Why this, why now

- **It is the lowest-risk Chapter D item by a wide margin.**
  Zero new code paths in the substrate. The audit chain
  already supports the read pattern (Phase 47 added
  `entries_range` for the Web UI's paginated viewer). All
  Phase 105 does is wire that read into a JSONL emitter on
  the CLI side.
- **It builds momentum for the rest of Chapter D.** Each
  later phase (MCP breadth, channel adapters, tool breadth,
  skills auto-creation) touches more substrate. Opening
  with a low-touch phase confirms the chapter rhythm
  matches the operator's posture and surfaces no surprises
  about the post-Phase-104 substrate state.
- **It closes the Hermes-comparison gap on trajectory
  export.** Hermes ships built-in trajectory generation for
  training next-gen tool-calling models. Aivyx's chain
  already contains everything such a pipeline would need;
  Phase 105 makes that data **dump-able** without
  decrypting redb by hand.
- **It does not depend on the held public-hosting
  decision.** Phase 61's `v0.1.0` publication is still
  paused. `aivyx audit export` lands against the existing
  build-from-source path and pays off immediately for any
  operator who wants to inspect or share their chain
  offline.
- **Export is read-only and offline by Q3.** The chain
  itself stays append-only; the export reads through the
  same encrypted-storage path as `--verify-only`, so it
  requires the passphrase and cannot be triggered remotely
  over the daemon socket. Forensic integrity is unchanged.

## Streak predictions

- **DESIGN.md** — **Will hold.** A read-only CLI emitter
  over existing audit data touches no locked technical-
  contract decision and adds no daemon-IPC variant (Q3
  resolution is offline-only). Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to fifty-two**.

- **PRODUCT.md** — **Will hold.** No P-* commitment
  touched; the export is third-party-tool ergonomics
  (operator can pipe the chain to whatever they want).
  Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to five** (was 4).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** Every line of Phase 105 lives in
  `aivyx-channel`'s binary (a new `CliMode::Audit`
  variant + parse block + an `aivyx_modules/audit_export.rs`
  module that emits JSONL via `serde_json::to_writer`).
  `aivyx-core` is not touched. Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to five** (was 4).

- **New workspace deps** — Zero. `serde_json` is already a
  workspace dep; `aivyx-audit` already exports
  `SignedEntry` with `Serialize`. The emitter writes one
  `to_writer` call per entry.

- **Test count** — Positive. New tests cover:
  per-entry JSONL render (round-trip through
  `serde_json::from_str`), the `--from` / `--limit`
  slicing, the empty-chain → empty-output edge case, and
  the CLI parse paths. Rough prediction: **+6 to +12**.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_105.md` + `docs/ROADMAP.md` Chapter D entry
refinement (flip Phase 105 row from scheduled to Active and
add a per-phase `## Phase 105` section) + `docs/README.md`
status row.

### Task 2 — `aivyx audit export` subcommand + JSONL emitter

- **CLI shape.** New `CliMode::Audit(AuditSubcommand)` with
  one sub-subcommand `Export { from: Option<u64>, limit:
  Option<usize> }`. Parse pattern mirrors `CliMode::Tool(
  ToolSubcommand::Init { ... })` from Phase 103. Both flags
  are optional; missing `--from` means "start at seq 0,"
  missing `--limit` means "no limit."
- **Cold-start storage open.** Reuse the existing
  cold-start path: derive the audit-chain key from the
  passphrase, open `RedbStorage` read-only, call
  `PersistentAuditLog::entries_range(from, limit)` to
  iterate. The export does **not** spawn the daemon; if
  the daemon is already running with the same encrypted
  store, `redb`'s reader-coexists-with-writer semantics
  let the export run concurrently.
- **JSONL emitter.** New `aivyx_modules/audit_export.rs`
  module. One pure function `render_line(&SignedEntry) ->
  Result<String, serde_json::Error>` that emits a
  newline-terminated JSON object with the full
  `SignedEntry` shape: `{seq, appended_at_ms, prev_mac
  (hex), mac (hex), event}`. The Q1(a) decision is to
  include `prev_mac` and `mac` so downstream tooling can
  re-verify the HMAC against a separately-supplied
  genesis seed; MAC bytes are emitted as lowercase hex
  for transport safety (JSON doesn't carry raw bytes
  cleanly).
- **Output destination.** stdout. Operator pipes (`>
  trajectory.jsonl` or `| jq .event.kind`). No
  `--output <path>` flag in v1 — adds complexity for no
  capability the shell doesn't already give.

### Task 3 — Tests + docs + exit

- **JSONL round-trip test.** Construct a small `Vec<
  SignedEntry>` (one entry of each `AuditEvent` variant),
  emit via `render_line`, parse each line back through
  `serde_json::from_str`, assert byte-equal to the
  source.
- **Empty-chain test.** An empty `Vec<SignedEntry>` emits
  zero bytes — no leading bracket, no trailing newline.
- **`--from` / `--limit` parse tests.** Bare `aivyx audit
  export` parses; `aivyx audit export --from 100 --limit
  50` parses; `aivyx audit export --from notanumber`
  errors at parse time; `aivyx audit` alone reports a
  usable error.
- **`docs/INSTALL.md`** — a one-paragraph mention beside
  the existing `aivyx --verify-only` documentation
  pointing at `aivyx audit export` as the export-side
  companion.
- **`docs/AUDIT_EXPORT.md`** (new) — short reference doc
  covering: the JSONL shape (one example line per
  `AuditEvent` variant), how to re-verify the chain
  downstream (the HMAC algorithm is documented in
  `docs/DESIGN.md` via the Phase 6 audit-chain design),
  and a worked `jq` example for extracting tool-call
  trajectories.
- Exit: ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Per-line schema:** (a) **Full `SignedEntry`
  projection** — `{seq, appended_at_ms, prev_mac (hex),
  mac (hex), event}`. Chosen so downstream tooling can
  re-verify the HMAC chain against a separately-supplied
  genesis seed. The size cost is ~96 bytes of hex per
  line, negligible vs. the typical event payload. Rejected
  the "minimal seq + appended_at_ms + event" form
  (saves bytes but loses the integrity story) and the
  "leading genesis-header line" form (would emit the
  audit-key genesis seed or its hash — a security
  decision worth its own future micro-phase, not a
  Phase 105 default).
- **Q2 — Filter set in v1:** (a) **Sequence-based only**
  — `--from <seq>` + `--limit <N>`. Maps directly to
  `PersistentAuditLog::entries_range(from, limit)`.
  Smallest Phase 105 scope; operators wanting time-range
  or session-correlated filtering pipe through `jq`.
  Rejected the seq + time form (would pull in a date-
  parsing dep or hand-roll RFC 3339 — a worthwhile add
  later, not on the opener) and the seq + time +
  correlation form (touches every `AuditEvent` variant to
  extract correlation ids — Phase 105's "easy wins
  first" framing argues against it).
- **Q3 — Source path:** (a) **Offline-only** (cold-start
  storage open). Same path as `aivyx --verify-only` — opens
  encrypted storage with the operator's passphrase, reads
  `KeyDomain::Audit`, emits, exits. Works whether the
  daemon is running or not. No new IPC variant — the
  Phase 41 protocol-negotiation surface stays
  byte-identical. Most secure of the three options: the
  export requires the passphrase and cannot be triggered
  remotely over the daemon socket. Rejected daemon-mode
  (new `Query::ExportAudit` IPC variant — protocol
  growth for no operator benefit on this phase) and
  both-with-fallback (twice the code paths for the same
  output).

## Exit criteria

- [ ] `docs/PHASE_105.md` + ROADMAP Chapter D Phase 105
  entry flip + docs/README status row — Task 1 (this
  commit).
- [ ] `aivyx audit export [--from <seq>] [--limit <N>]`
  subcommand wired through `CliMode::Audit(
  AuditSubcommand::Export)` — Task 2.
- [ ] `aivyx_modules/audit_export.rs` module with the
  pure `render_line` emitter + cold-start storage open
  driver — Task 2.
- [ ] JSONL round-trip test covering one entry per
  `AuditEvent` variant — Task 3.
- [ ] Empty-chain test (zero entries → zero bytes) —
  Task 3.
- [ ] CLI parse tests (happy path + invalid `--from` +
  `aivyx audit` bare error) — Task 3.
- [ ] `docs/AUDIT_EXPORT.md` new + `docs/INSTALL.md`
  mention — Task 3.
- [ ] ROADMAP + docs/README refreshed at exit — Task 3.
- [ ] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [ ] DESIGN.md streak extends to fifty-two.
- [ ] PRODUCT.md streak extends to five.
- [ ] Production-core `lib.rs` streak extends to five.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive — predicted `+6` to
  `+12`.
- [ ] Zero clippy warnings.

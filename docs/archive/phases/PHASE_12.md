# Phase 12 — `web.fetch` + streaming tool output (second product phase)

**Status:** Active (opened 2026-04-15). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–11 precedent.
**Predecessor:** [PHASE_11.md](PHASE_11.md) (exit commit `16422e2`,
hash backfill `8c12bc6`)
**Contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **eleven phases running** at
Phase 12 entry, target **twelve** at Phase 12 exit)

## Goal

Phase 12 is the **second product phase**. Phase 11 proved the Role
primitive works by shipping `shell.exec` as the first dangerous
tool and threading the active role through the turn loop; Phase 12
is the phase that proves the *pattern* generalizes by shipping a
structurally distinct second tool (`web.fetch`) and the
infrastructure that tool justifies (streaming tool output via
`StreamEvent::ToolOutput`).

The headline outcome: when Phase 12 closes, the `researcher` role
shipped in Phase 11 stops being a "contrast role that proves
allowlists work" and starts being a role the user would actually
run. A researcher agent can fetch a URL, the response body streams
into the renderer incrementally (not as one bundled blob at
finish), the URL is matched against a `web.fetch:url-prefix:<...>`
capability scope, and a `coder`-role call to the same tool is
rejected at the allowlist gate. Phase 11's `shell.exec:cwd:<path>`
scope pattern is shown to generalize to a non-filesystem
capability scope. The `StreamEvent::ToolOutput` variant — a
Phase 11 Q2 deferral — is promoted to primary task, because the
Phase 11 "one more streaming-output tool away" design pressure
is exactly what `web.fetch` delivers.

Like Phase 11, Phase 12 threads one new primitive (streaming tool
output) through every subsystem that observes tool lifecycle
(turn loop, renderer, channel adapters, audit bridge), then
ships exactly one consumer of that primitive (`web.fetch`), and
freezes. The task count is the same five, but Phase 12 is
deliberately smaller than Phase 11 in raw scope — there is no
new config primitive, no new capability-layer concept, no new
catalog-filter enforcement point. The Phase 11 seams are reused
as-is; Phase 12's job is to show they *can* be reused as-is
without design rework.

## Why now

Three structural reasons:

1. **Phase 11's deferrals give Phase 12 its shape.** Phase 11
   closed with three net-new deferrals, one of which
   (`StreamEvent::ToolOutput`) was explicitly tagged "earliest
   plausible: whichever phase ships the second streaming-output
   tool." Phase 12 is that phase by construction — the deferral
   isn't reopened speculatively; it's promoted because the
   design pressure it was waiting for has arrived.
2. **The `researcher` role needs a tool.** Phase 11 shipped
   `researcher` as a contrast role whose primary job was to
   prove the allowlist gate works: the role's allowlist
   deliberately excludes `shell.exec`, and the regression test
   asserts a `researcher`-role `shell.exec` call is denied at
   the allowlist gate (not the capability gate). The role
   currently exists *only* to be denied. Phase 12 gives it a
   tool it can actually use, which upgrades it from a
   test-shaped role to a user-shaped one.
3. **Generalization pressure on the capability layer.**
   Phase 11's `shell.exec:cwd:<canonicalized-path>` scope
   shape was engineered specifically around filesystem path
   semantics. Whether the same prefix-attenuation idea
   generalizes to non-filesystem scopes is a design claim
   Phase 11 made but did not test. `web.fetch:url-prefix:<...>`
   exercises the same idea against a structurally different
   namespace (scheme + host + path, with the scheme+host part
   requiring an *exact* match — see Q4). If the generalization
   holds, Phase 13+ tools can reuse the pattern without
   re-proving it; if it fails, the failure mode is recorded
   here and not propagated.

## Non-goals

- **No HTTP write verbs.** `web.fetch` is **GET-only** in Phase
  12 (Q1 pinned at phase open). No POST, PUT, PATCH, DELETE,
  HEAD, OPTIONS. Write verbs introduce a categorically different
  audit story (idempotence, side effects on remote state, CSRF
  token handling) that Phase 12 does not need to solve.
  `web.post` or `web.put` are candidates for a future phase if
  a use case drives them.
- **No response headers surfaced to the model by default.**
  (Q3 pinned at phase open.) Response headers are recorded in
  the audit log as part of `ToolCallFinished`, but the tool's
  model-visible return value is `(status_code, body)` only.
  Leaking `Set-Cookie`, `Authorization`, or `Proxy-Authenticate`
  into the model's context window is a footgun even for a
  Trusted-tier agent, and "opt in to exposing headers" is a
  later phase's problem.
- **No redirect following across scopes.** Phase 12 will either
  disallow redirect following entirely or only follow redirects
  whose target URL *also* matches an active `web.fetch:url-
  prefix:` scope held by the caller. Following a 302 from
  `https://example.com/` to `https://evil.com/` without a
  capability check is a classic capability-escape vector and is
  explicitly out of scope. Final decision in Q5 under Task 2.
- **No response body size limit configuration.** Phase 12 ships
  a hard-coded upper bound on response body bytes (exact number
  TBD in Task 2 — likely 10 MiB as a starting point). Making
  it configurable is a Phase 13+ concern if a use case needs
  larger fetches. Undersized hard limit → clear error; no
  silent truncation.
- **No retries, no connection pooling tuning, no custom TLS
  roots.** Phase 12 uses `reqwest`'s defaults for everything
  not explicitly overridden. `reqwest` is already a workspace
  dependency (used by `aivyx-llm` for the Anthropic client),
  so adding `web.fetch` is a zero-new-dep change.
- **No `ToolOutput` audit-log fan-out.** Streaming chunks are
  a rendering concern. The audit bridge continues to record one
  `ToolCallFinished` entry per tool call, with the aggregated
  body as part of its payload. Per-chunk audit events would
  balloon the audit chain for no forensic gain — the invariant
  "one audit entry per tool call" is load-bearing for the
  `--verify-only` forensic walker and Phase 12 does not break
  it.
- **No speculative reopening of Phase 11 deferrals.** The
  `ToolOutcome::NotInRole` typed variant (Phase 11 Q1) and the
  second regression channel (Phase 11 Q6) stay deferred unless
  Phase 12 implementation work surfaces concrete evidence that
  they're needed. This is deliberate: the "reopen on evidence,
  not speculation" discipline is what keeps the foundation
  backlog from growing into a wish list.
- **No DESIGN.md edits.** `StreamEvent::ToolOutput` is a new
  variant on an existing `enum` — the DESIGN.md code block is
  illustrative (per the Phase 11 handoff rule from `phase_11_
  handoff.md`), not byte-exact. `web.fetch` fits inside the
  existing D3 tool trait and D2 capability scope deliverables
  without contract changes. If mid-phase work surfaces a
  contract conflict, that's an amendment under
  `docs/amendments/` (directory still doesn't exist), not a
  silent edit.
- **No `aivyx-cli` crate** (unchanged from Phase 11). The tool
  surface grows; the CLI surface does not.

## Entry criteria (all met from Phase 11 exit)

- [x] Phase 11 is frozen. Exit commit `16422e2`, hash backfill
      `8c12bc6`. `docs/README.md` phase-status table reflects
      both.
- [x] `cargo test --workspace` is **422 green** (verified at
      Phase 12 entry: 2026-04-15). Phase 11 delivered +55 net
      tests against the 367-test Phase 11 entry baseline.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      is clean.
- [x] DESIGN.md byte-identical to `e0d6437`. Streak at **eleven**
      (verified: `git diff e0d6437 HEAD -- DESIGN.md | wc -l ==
      0`).
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to `16422e2`
      (the Phase 11 re-baseline after Task 3 broke the previous
      streak for `ShellExecTool`). Production-core streak at
      **one phase** and expected to break in Phase 12 Task 1 —
      the break is an intentional phase-scope decision tied to
      the `StreamEvent::ToolOutput` variant.
- [x] Foundation backlog carries **three rolling items** from
      Phase 11: streaming tool output (to be consumed as the
      driver of Task 1), `ToolOutcome::NotInRole` (held unless
      evidence surfaces), second regression channel (held
      unless evidence surfaces).
- [x] Pre-commit hook (`scripts/pre-commit.sh`) runs `cargo
      clippy` workspace-wide with `-D warnings` before every
      commit. Held across all of Phase 11 with zero regressions.
- [x] `reqwest` is already in the workspace dependency graph
      (via `aivyx-llm`) — `web.fetch` does not need a new crate.
      Zero-new-dep streak enters Phase 12 still held.

## Open questions (pinned at phase open unless marked otherwise)

Six questions. Q1 and Q3 are **pinned at phase open** because
their resolution narrows Phase 12's scope decisively. Q2, Q4,
Q5, and Q6 are **held open** for task-level resolution because
they're implementation details that benefit from being decided
against real code.

### Q1 — HTTP verbs: GET-only, or GET+HEAD, or full read verbs?

**PINNED: GET-only.** `web.fetch` in Phase 12 accepts only the
GET verb. HEAD would be cheap to add but invites "why not
OPTIONS too, why not conditional GETs" scope creep. If a
concrete use case for HEAD surfaces mid-phase, it lands as a
task-local decision recorded in the relevant task block; it is
not a reason to broaden the phase goal. Write verbs (POST/PUT/
PATCH/DELETE) are categorically out (non-goals section).

### Q2 — How do streaming chunks observe the `CancellationToken`?

**HELD OPEN, task-level.** Task 1 must ensure that a Ctrl-C
during an in-flight tool call propagates to the chunk emitter
so the tool yields promptly. The turn loop already holds a
`CancellationToken`; the question is whether it threads into
`Tool::run` or whether chunk emitters poll a shared handle.
Probable answer: `Tool::run` already receives a
`CancellationToken` argument, so this is a plumbing check, not
a design decision. Resolved during Task 1 implementation.

### Q3 — Are response headers model-visible or audit-log-only?

**PINNED: audit-log-only.** Response headers are recorded in
the audit entry's `ToolCallFinished` payload but are **not**
returned to the model. The model sees `{status: u16, body:
String}`. This is the cautious default; "opt in to exposing
specific headers" is a later-phase concern. The audit log
being the only place headers land is a forensic-story win too
— if a response header ever turns out to have leaked something,
the audit chain records what was fetched and when.

### Q4 — URL-prefix scope matching: how strict?

**HELD OPEN, task-level (Task 2).** The naive answer ("string
prefix") is wrong: `https://example.com/` would "match"
`https://example.com.evil.com/` because the latter's raw bytes
start with the former's. The correct shape is **(scheme, host)
exact match** followed by **path prefix match** on the
canonicalized path portion only. Port must also match
exactly if specified in the scope. Query-string and fragment
are **not** part of the match — scope `https://api.example.com/
v1/` matches `https://api.example.com/v1/users?id=42`.
Canonicalization edge cases (trailing slash, percent-encoding,
case folding of host) decided in Task 2 against the `url` crate
— which is also already in the workspace via `reqwest`, so
still zero new deps.

### Q5 — Redirect following.

**HELD OPEN, task-level (Task 2).** Two candidate resolutions:
**(a)** redirect following is **off by default**. `web.fetch`
receives a URL, issues exactly one HTTP request, and returns
whatever the server sends — 3xx responses surface as
`{status: 302, body: ""}` to the model with the `Location`
header visible in the audit log only (per Q3). The agent can
then choose whether to fetch the redirect target as a separate
tool call (which will re-check the capability layer naturally).
**(b)** redirect following is **on, but gated**: `reqwest`'s
redirect policy is replaced with a custom one that re-checks
each redirect hop against the caller's
`web.fetch:url-prefix:` scopes and bails the instant a hop
target isn't covered. Leaning toward **(a)** because it's
simpler and puts redirect decisions in the model's context
rather than hiding them in the transport layer, but the
ergonomics of forcing the agent to fetch-redirect-fetch every
time might argue for (b). Decide in Task 2 against real
implementation feel.

### Q6 — `StreamEvent::ToolOutput` chunk payload shape.

**HELD OPEN, task-level (Task 1).** `StreamEvent<'a>` is
borrowed (`&'a str`, `&'a Value`, etc. — verified at
`crates/aivyx-core/src/lib.rs:209`), so the new variant should
also be borrowed. Two sub-questions: **(a)** is the chunk a
`&'a str` or a `&'a [u8]`? Body is `String` for text
responses but `web.fetch` could conceivably receive binary
content; probably `&'a str` with the tool responsible for
rejecting non-UTF-8 bodies at a size-prefix check. **(b)**
does the variant carry the `tool_name` and `tool` id that
`ToolCallStarted` / `ToolCallFinished` both carry, for
renderers that want to interleave chunks from concurrent tool
calls? Phase 12 has no concurrent-tool-call story, so the
pressure for `tool_name` on the chunk variant is zero today,
but including it now is cheap and future-proofs against the
day concurrency lands. Resolved in Task 1 by writing the
variant.

## Draft task breakdown

Five tasks. Ordered so the streaming-output primitive lands as
pure infrastructure first (Task 1), then is consumed by
`web.fetch` (Task 2), then role wiring is verified against the
existing Phase 11 seams (Task 3), then frozen (Task 5). Task 4
is the working-session slot — reserved for whatever
mid-implementation correction the phase surfaces that doesn't
fit cleanly into Tasks 1–3. Each task gets a working-session
commit and closes before the next opens — the Phase 7–11
cadence.

### Task 1 — `StreamEvent::ToolOutput` variant + turn-loop emission + renderer wiring

**What lands:**

- New `StreamEvent<'a>` variant:
  ```
  ToolOutput {
      tool: ToolId,
      tool_name: &'a str,
      chunk: &'a str,
  }
  ```
  Borrow lifetime matches the existing variants (verified at
  `crates/aivyx-core/src/lib.rs:209`). Carrying `tool_name`
  and `tool` on every chunk is intentional — it keeps the
  variant consistent with `ToolCallStarted` / `ToolCallFinished`
  and it future-proofs against a concurrent-tool-call world
  (Q6). `chunk: &'a str` restricts streaming to UTF-8 bodies;
  binary streaming is a later-phase concern.
- Turn-loop emission path: `Tool::run` implementations that
  want to stream gain a mechanism to emit `ToolOutput` events
  during execution, not just at finish. Exact mechanism
  resolved in Task 1 — candidates include (a) passing a
  `StreamEventSink` handle into `Tool::run`, (b) extending
  `ToolResult` with a streaming variant, (c) a channel-based
  sender closed over at dispatch. Leaning toward (a) because
  it's the smallest change to the `Tool` trait and keeps the
  dispatch layer unchanged. Note that the trait signature
  change **may** break the production-core byte-identity
  streak — that's a Phase 12 expected break, same shape as
  Phase 11 Task 3's `ShellExecTool` re-export.
- `LocalChannel` renderer: format incoming `ToolOutput` chunks
  as inline text without adding a per-chunk marker, so the
  user sees a streaming response body as-it-arrives. Each
  chunk prints verbatim; the existing tool-start / tool-end
  markers bracket the streamed region. Match the visual shape
  of the existing `TextChunk` stream so a streamed tool output
  reads like streamed LLM text.
- `TelegramChannel` renderer: Telegram has message-length
  limits and no partial-message editing is assumed in Phase
  8's adapter. Probable resolution: **buffer chunks locally**
  in the Telegram adapter and emit one finalized message at
  the tool's finish event — i.e., Telegram sees streaming as
  a no-op and gets the bundled behavior it has today. This
  keeps the trust-tier asymmetry pattern from Phase 11: Local
  gets the richer UX, `SemiTrusted` gets the safer default.
  If that resolution holds, Telegram's changes in Task 1 are
  minimal (observe the variant, accumulate, render at finish)
  — and importantly it means Telegram users don't need to
  wait for a later phase to get `web.fetch`, they just get it
  without the streaming UX.
- Audit bridge: recognizes `ToolOutput` as "pass-through, do
  not log." The audit chain continues to record one entry per
  `ToolCallFinished`. A regression test asserts that N chunks
  produce zero new audit entries, and the single
  `ToolCallFinished` entry still fires exactly once with the
  aggregated body.
- A **scripted test tool** (working name
  `ScriptedStreamingTool` under `aivyx-core` test-support
  scaffolding) that emits a configured sequence of chunks
  followed by a finish result. This tool is **not registered**
  in any production channel — it exists purely to give Task 1
  an integration-test subject without creating a dependency
  on Task 2's real `web.fetch` tool. Task 1 lands end-to-end
  through turn loop + renderer + audit bridge + regression
  test using only the scripted tool.

**Acceptance:**

- `StreamEvent::ToolOutput` variant present, pattern-matched
  everywhere `StreamEvent` is exhaustively matched (compiler
  enforces this).
- Regression test: scripted streaming tool emits three
  chunks, `LocalChannel` writer captures all three in
  order between the tool-start and tool-end markers.
- Regression test: same scripted tool, `TelegramChannel`'s
  scripted transport captures one bundled message at finish,
  no intermediate messages.
- Regression test: same scripted tool, audit chain records
  exactly one `ToolCallFinished` entry, verify() passes.
- `cargo test --workspace` green. Test delta **≥ +5**.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- Production-core byte-identity streak **broken** here (new
  `StreamEvent` variant), recorded in the ship log, re-
  baseline at exit.

**Why Task 1 is pure infra and not bundled with `web.fetch`:**
separating the variant from its first consumer keeps each
review surface small. The `StreamEvent::ToolOutput` change
touches every crate that observes `StreamEvent` exhaustively
(audit bridge, two renderers, turn loop); `web.fetch` touches
the tool registry, the capability layer, and the `reqwest`
call path. Landing them together would conflate two
independent risks. Phase 11 Task 3 bundled the validator
extension with `shell.exec` because the validator extension
had no meaningful test subject without the tool — here, the
scripted streaming tool gives Task 1 a real test subject that
doesn't depend on Task 2.

### Task 2 — `web.fetch` tool at `TrustTier::Trusted` + `SemiTrusted`

**What lands:**

- New `WebFetchTool` under `crates/aivyx-core` (same crate
  home as `ShellExecTool` — keeps trust-tier-aware
  registration discoverable in one place). Registered by a
  `build_web_fetch_for_channel(trust_tier: TrustTier)`
  helper analogous to `build_shell_exec_for_channel`. Unlike
  `shell.exec`, the helper returns the tool for **both**
  `Trusted` and `SemiTrusted` tiers — network fetches are
  not inherently dangerous in the way process spawns are,
  and a Telegram-attached agent with a `researcher` role
  should be able to fetch URLs. `Untrusted` and `Kernel`
  tiers do not get the tool (reserved for future adapters
  that may need stricter defaults).
- Input schema: `{url: string, timeout_ms?: u32}`. Flat
  shape — does not need Phase 11 Task 3's nested-object
  validator extension (which is a nice side signal that the
  validator's flat-object path was the right default).
  `timeout_ms` is optional with a sensible upper bound (TBD:
  probably 30s). Schema validation exercises the already-
  shipped validator exactly as-is.
- Capability scope shape: `web.fetch:url-prefix:<URL>`.
  Matching is (scheme, host, port) exact + path-prefix on the
  canonicalized path. Normalization via the `url` crate (zero
  new deps — already in tree via `reqwest`). Q4 resolves
  the canonicalization edge cases here.
- Registration-time defaults: the default `coder` role's
  allowlist does **not** include `web.fetch` (coder uses
  `shell.exec`). The default `researcher` role's allowlist
  **does** include `web.fetch`. Capability grants in the
  default config give the `researcher` role exactly one
  `web.fetch:url-prefix:` scope for a well-known example URL
  (probably `https://httpbin.org/` as an intentionally
  boring target — easy to smoke-test, no risk of smoke tests
  scraping anything sensitive).
- GET-only (Q1). No verb field in the schema. The tool
  issues `reqwest::Client::new().get(url)` unconditionally.
- Response body streamed via Task 1's `StreamEvent::ToolOutput`
  path. Hard body size limit enforced at dispatch: exceeding
  the limit surfaces as a tool error with a clear message,
  not a silent truncation.
- Redirect policy: resolved per Q5 against real code. Default
  assumption going in is (a) **no redirect following** —
  `reqwest::redirect::Policy::none()` — with 3xx responses
  surfaced as-is.
- Response headers are recorded in the audit log as part of
  the `ToolCallFinished` payload (Q3), but the tool's
  return value to the model is `{status: u16, body: String}`.
  Per-header audit shape TBD in implementation — probable
  answer is a `BTreeMap<String, String>` on the existing
  `ToolCallFinished` event payload, but that may need a
  separate audit-layer edit if the current payload shape
  can't accommodate it.

**Acceptance:**

- `web.fetch` registered for `Trusted` and `SemiTrusted`
  tiers, not for `Untrusted`/`Kernel`. Regression test
  exercises all four tiers.
- Regression test: `researcher` role + granted
  `web.fetch:url-prefix:https://httpbin.org/` scope, fetches
  `https://httpbin.org/get`, response body streams through
  `StreamEvent::ToolOutput`, final result is returned.
- Regression test: **capability-gate denial** — same
  scripted fetch against a URL *not* covered by the granted
  scope (e.g., `https://example.com/`) is denied at the
  capability layer, routed through `ToolOutcome::Denied`.
- Regression test: **allowlist-gate denial** — `coder` role
  (whose allowlist does not include `web.fetch`) attempts
  `web.fetch`, denied at the allowlist gate (not the
  capability gate), routed through Phase 11's synthetic
  `tool.allowlist:web.fetch` scope. Mirrors Phase 11 Task 4's
  dual-denial regression shape.
- Regression test: **URL-prefix attenuation** — agent holds
  `web.fetch:url-prefix:https://example.com/`, fetches
  `https://example.com.evil.com/`, denied. The naive-string-
  prefix-bug regression test. Critical: this test is the
  one that asserts the Q4 resolution is implemented
  correctly.
- Regression test: response body exceeds hard size limit,
  tool surfaces a clear error rather than truncating or OOMing.
- `cargo test --workspace` green. Test delta **≥ +10**.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- Production-core byte-identity: may break here if
  `WebFetchTool` needs to be re-exported at the crate root
  (same shape as `ShellExecTool` in Phase 11 Task 3). If so,
  record the break; if not, leave the re-baseline in Task 1.

### Task 3 — `researcher` role upgrade + default config wiring

**What lands:**

- Default `researcher` role's `tool_allowlist` gains
  `web.fetch`.
- Default `researcher` role gets one `web.fetch:url-prefix:`
  capability grant in the default `aivyx.toml` (the
  `https://httpbin.org/` scope mentioned in Task 2, or
  whatever Task 2 settles on).
- Cross-role regression test: same physical TOML config
  produces two working agents — a `coder` with `shell.exec`
  and no `web.fetch`, a `researcher` with `web.fetch` and no
  `shell.exec` — both launched from the same `aivyx` binary
  via `--role <name>`, each denied the other's tool at the
  allowlist gate. This is the phase-level "roles actually
  work for real product tools, not just for shell.exec"
  proof.
- Possibly a small docs touch-up to the Phase 11 example
  config shipped under `examples/` (or wherever it lives —
  TBD at task entry) to reflect the new researcher
  capabilities.

**Acceptance:**

- Cross-role regression test green.
- Default config loads cleanly with the new grants.
- `cargo test --workspace` green. Test delta **≥ +2**.
- Production-core lib.rs byte-identical across Task 3
  (no trait changes — Task 3 is pure config and test).

**Note:** Task 3 is the smallest task in the phase by design.
Most of the work is already done by Tasks 1 and 2; Task 3 is
just the config wiring and the cross-role regression. If
Task 3 grows beyond a half-day of work, that's a signal
something went wrong in Task 2's seam design and should be
investigated rather than absorbed into Task 3.

### Task 4 — Working-session slot (contingent)

**Purpose.** Reserved for whichever mid-implementation
correction Phase 12 surfaces. Explicit candidates, in order
of likelihood:

1. **Audit-payload widening** for `ToolCallFinished` if Task 2
   discovers the existing payload shape can't carry response
   headers cleanly. This is the most plausible Task 4 use.
2. **URL canonicalization deep dive** if Q4 turns out to
   have more edge cases than Task 2 can absorb without
   ballooning. `url` crate's matching semantics may surface
   surprises under IDN or percent-encoding.
3. **Redirect policy rework** if Q5's default (a) turns out
   to be too painful in practice and (b) lands instead.

If none of these surface, **Task 4 is dropped** and the
phase ships with four tasks plus the freeze, not five. This
is the deliberate-slack discipline: having a named slot for
the correction is not the same as being obligated to fill it.
Phase 10 Task 3's emission-gap close was exactly this kind of
slot use; Phase 11 did not need one.

**Acceptance (if used):**

- Whatever the correction is, it lands with tests and passes
  clippy.
- The Task 4 ship record documents *why* the slot was used
  and which alternatives were considered.

**Acceptance (if dropped):**

- Task 5 exit-freeze records "Task 4 not used" explicitly.
  The phase does not silently renumber Task 5 to Task 4 —
  the slot existed and is recorded as unused, so the phase
  history matches the draft.

### Task 5 — Exit freeze

Standard docs-only freeze, mirroring Phase 9 Task 6 / Phase 10
Task 4 / Phase 11 Task 5 exactly.

**What lands:**

- Task 1–4 ship records appended above.
- Final exit-criteria checklist at the bottom of this
  document.
- `docs/README.md` phase-status table flipped to Phase 12
  Frozen (hash backfilled in a follow-up commit per the
  phase-discipline workflow memory).
- `docs/ROADMAP.md` — Phase 12 entry removed, Phase 13
  entry scaffolded with whatever Phase 12 learned.
- "Decisions made during Phase 12 that aren't in DESIGN.md"
  block recording the Q1/Q3 pins plus the Q2/Q4/Q5/Q6
  resolutions from tasks 1 and 2.
- "Phase 12 deferrals" block enumerating whatever net-new
  deferrals the phase produces. Phase 12 entered with three
  rolling deferrals from Phase 11; Task 1 consumes one
  (streaming tool output). Net exit delta TBD.

**Commit message:** `docs(phase-12): exit freeze — second
product phase complete` (or similar — match the Phase
11 shape).

**Guardrails:** docs-only. No source edits, no Cargo.toml
edits, no test edits. If the freeze surfaces a latent issue,
it becomes a follow-up commit after the freeze, not a bundle.

## Success metrics (draft)

- **422 → ~440 tests minimum.** Task 1 adds ≥5, Task 2
  adds ≥10, Task 3 adds ≥2, Task 4 adds 0+ (contingent).
  Net Phase 12 delta ≥+17 against the 422 entry baseline,
  well above the ≥+10 heuristic. Actual delta reported at
  freeze.
- **DESIGN.md streak at 12.** Verified by `git diff e0d6437
  HEAD -- DESIGN.md | wc -l == 0`.
- **Production-core byte-identity streak re-baselined at
  Phase 12 exit.** Task 1's new `StreamEvent` variant is
  the expected break point.
- **Zero-new-dep streak held.** Every dependency Phase 12
  needs (`reqwest`, `url`) is already in-tree.
- **The researcher role is a role you'd actually run.** This
  is a product metric, not a test metric, and it's measured
  by whether the Phase 12 exit regression test set includes
  a non-trivial `researcher`-role scenario that looks like
  "an agent fetching something useful" rather than "an
  agent being denied something."

## Rolling reference material

- **Phase 11 handoff memory** (`memory/phase_11_handoff.md`) —
  Phase 12 inherits all of this, because Phase 11's seams
  are the seams Phase 12 is building on. Relevant sections:
  `Tool::input_schema()` predates Phase 6, validation order,
  dispatch-layer session/prefix injection.
- **Phase 11 deferrals block** (`docs/PHASE_11.md`, under
  "### Phase 11 deferrals") — the three items Phase 12
  inherits, one of which becomes Task 1.
- **`DESIGN.md` D2 capability scopes** — unchanged since
  `e0d6437`. The `web.fetch:url-prefix:<URL>` shape fits
  inside D2 without an amendment.
- **`DESIGN.md` D3 tool trait** — unchanged. `Tool::run`
  may gain a streaming sink argument in Task 1; per the
  Phase 11 handoff rule, adding a parameter to a trait
  method whose DESIGN.md block is illustrative is not a
  streak break.

---

## Task 1 — correction recorded mid-implementation (2026-04-15)

**What the draft assumed:** Task 1 would need to extend the
`Tool` trait with a streaming sink parameter — the draft listed
three candidate mechanisms (pass a `StreamEventSink` handle into
`Tool::run`, extend `ToolResult` with a streaming variant, or a
channel-based sender closed over at dispatch) and leaned toward
(a). The draft also expected this to "**may** break the
production-core byte-identity streak" on top of the new
`StreamEvent` variant.

**What the code actually shows:** the seam already exists.
`Tool::execute` at `crates/aivyx-core/src/lib.rs:503` already
takes `context: &ToolContext<'_>`, and `ToolContext` at line
513 already carries `channel: &'a dyn ChannelContext`.
`ChannelContext::stream_event` at line 171 already accepts
`StreamEvent<'_>`. A streaming tool emits incremental output by
calling `context.channel.stream_event(StreamEvent::ToolOutput
{ tool, tool_name, chunk: &s }).await` inside its `execute`
body — identical to the planner's text-streaming pattern at
`llm_planner.rs:246` where the planner emits
`channel.stream_event(StreamEvent::Text(chunk)).await` inside
its own step loop.

**Scope change.** Task 1 no longer touches the `Tool` trait at
all. It does not add a `StreamEventSink`, does not extend
`ToolResult`, does not pass any new argument anywhere. The
entire Task 1 delta is:

1. Add `StreamEvent::ToolOutput { tool, tool_name, chunk }`
   variant to the enum in `aivyx-core/src/lib.rs`.
2. Extend the three exhaustive `match StreamEvent` sites to
   handle it:
   - `crates/aivyx-channel/src/render.rs:91-108` (Local
     renderer — inline pass-through, no marker).
   - `crates/aivyx-telegram/src/telegram_channel.rs:134-169`
     (Telegram — buffer-and-ignore so finalized text lands at
     `ToolCallFinished` per the trust-tier asymmetry rule).
   - `crates/aivyx-core/src/agent.rs:761-776` (the
     `RecordedEvent` shadow enum used by the agent-level
     fake-channel tests).
3. Ship a `ScriptedStreamingTool` under a test-support module
   of `aivyx-core` that drives streaming emission through an
   integration test. The tool itself is **not** registered in
   any production channel — it exists purely as a test subject.
4. Add regression tests for: (a) Local channel captures all
   chunks in order between start/finish markers, (b) audit
   chain records exactly one `ToolCallFinished` entry for N
   chunks (the invariant the draft's success metric names
   as load-bearing).

**Production-core streak break: still expected, for a
smaller reason.** The `StreamEvent` variant addition is the
only break, not a trait signature change. Per the Phase 11
handoff rule (DESIGN.md code blocks are illustrative, not
byte-exact), this remains a legitimate streak break to
record in the ship log rather than an amendment trigger.
Re-baselined at the Phase 12 exit commit as normal.

**Telegram rendering: deferred to implementation.** The
draft said "buffer chunks locally and emit one finalized
message at the tool's finish event." That's still the
direction, but the actual Telegram renderer at
`telegram_channel.rs:134-169` shows every match arm is
effectively stateless — it turns a `StreamEvent` into an
async call with no per-tool-call accumulator state. Adding
per-tool-call buffering would need a new `HashMap<ToolId,
String>` on the channel struct, which is more surface than
the "Telegram gets a no-op rendering" one-liner implied.
**Simpler resolution:** Telegram's `ToolOutput` arm is a
no-op return (`Ok(())`), and the existing
`ToolCallFinished { outcome_summary, .. }` arm already
renders the final tool result. Users on Telegram see the
same finish-time summary they see today. Chunks are just
dropped. This matches the Phase 11 trust-tier asymmetry
pattern exactly (Local gets richer UX, SemiTrusted gets
unchanged-from-today) and avoids adding a per-tool-call
accumulator to the Telegram adapter. Recorded here so the
Task 1 ship log doesn't have to re-justify it.

**Q6 resolved (early):** the `StreamEvent::ToolOutput`
variant shape is:
```
ToolOutput {
    tool: ToolId,
    tool_name: &'a str,
    chunk: &'a str,
}
```
Borrowed string chunk (UTF-8 only), matching the borrow
lifetime pattern of the other variants. `tool` and
`tool_name` included for symmetry with `ToolCallStarted` /
`ToolCallFinished`, future-proofing against a concurrent-
tool-call world. `&'a str` rather than `&'a [u8]` restricts
streaming to UTF-8 text; binary streaming is out of scope
for Phase 12 and is a later-phase concern.

---

## Task 2 — correction recorded mid-implementation (2026-04-15)

**What the draft assumed:** Task 2's scope shape would be
`web.fetch:url-prefix:<URL>`, implying a new `web.fetch` base
in `KNOWN_BASES` alongside the existing `net.fetch`.

**What the code actually shows:** `net.fetch` is already in
`KNOWN_BASES` (`aivyx-capability/src/lib.rs:42`), already
dispatches URL-prefix matching via `QualifierKind::UrlPrefix`
(`lib.rs:181-195`), and is already a member of both
`CEILING_TRUSTED` and `CEILING_SEMITRUSTED`. Adding a
separate `web.fetch` base would duplicate all three and leave
the existing `net.fetch` base orphaned (no consumer outside
tests). So the Task 2 scope shape is **`net.fetch:<URL>`**,
not `web.fetch:url-prefix:<URL>`. The **tool name** remains
`web.fetch` (the user-facing convention from the draft —
tool name and scope base are independent identifiers).

**Q4 resolved against real code.** The existing
`QualifierKind::UrlPrefix` matcher uses raw
`needed_q.starts_with(held_q)`
(`aivyx-capability/src/lib.rs:195`). That is exactly the
classic-string-prefix bug the Task 2 acceptance test is
supposed to catch: held `net.fetch:https://example.com/`
incorrectly grants needed `net.fetch:https://example.com.evil.com/`
because the needed string literally starts with the held
string. The existing `rule3_url_prefix_match` test at
`lib.rs:469` only covers `api.example.com/v1/users` vs
`api.example.com/` and never exercises the hostile-suffix
case.

**Fix in Task 2:** replace the `starts_with` body of
`QualifierKind::UrlPrefix::check` with a URL-aware matcher
that parses both sides via `url::Url` (transitively in tree
through `reqwest`) and returns `true` iff (scheme, host, port,
is_default_port) exactly match AND the canonicalized path of
`needed` starts at a **component boundary** under the
canonicalized path of `held`. "Component boundary" means
path-prefix via `std::path::Path::starts_with`-style
segment comparison, not byte-prefix, so
`/users/` does not admit `/users2/`. Trailing-slash
normalization: `/foo` and `/foo/` are treated as the same
directory for the purposes of prefix matching (a held
`example.com/api` grants needed `example.com/api/v1/users`).

**Blast radius of the capability-layer fix.** Grep for
`net.fetch:` across the workspace shows only in-crate test
fixtures in `aivyx-capability/src/lib.rs`. No agent config,
no production ceiling derivation, no audit fixture holds a
qualified `net.fetch:` scope. So the matcher rewrite is a
pure capability-layer edit with its own regression test
(hostile suffix denied, legitimate subpath still granted)
and a keep-green run of `rule3_url_prefix_match` +
`dispatch_url_with_comma_in_query_stays_url`.

**Production-core lib.rs streak:** still at risk. The tool
registration shape mirrors `ShellExecTool`'s
(`pub use tools::web_fetch::...` re-export at crate root),
so `lib.rs` gains at least one `pub use` line and the
byte-identity streak breaks for Task 2 as Task 1 already
broke it. Re-baseline at exit.

**Untouched Task 2 shape otherwise:** GET-only, body streamed
through `StreamEvent::ToolOutput`, 10 MiB hard body cap,
`reqwest::redirect::Policy::none()`, return value
`{status, body}` (headers audit-log-only). All the other
acceptance tests in the draft still apply verbatim — the
only drift is the scope base name and the URL-matching
fix.

---

## Task 3 — correction recorded mid-implementation (2026-04-15)

**What the draft assumed:** Task 3 would update a shipped
"default config file" (an `examples/aivyx.toml` or
equivalent) to add `web.fetch` to the `researcher` role's
`tool_allowlist` and grant one narrow `net.fetch:...` scope
to that role.

**What the code actually shows:** there is no shipped default
config file. `aivyx-config` synthesizes a **single** implicit
role named `"default"` (see `DEFAULT_ROLE_NAME` at
`crates/aivyx-config/src/lib.rs:193`) with
`ToolAllowlist::AllowAll`, built from legacy top-level fields
for backwards compatibility when a loaded config defines zero
roles. The `coder` and `researcher` role names appear only as
**test fixtures** inside `aivyx-config/src/tests.rs:802` —
they are not compiled-in defaults, not shipped in a TOML
file, and not referenced anywhere in the binary's runtime
code path. A `Glob **/aivyx.toml*` across the tree returns
zero matches.

**Scope change.** Task 3 does NOT create a new
`examples/aivyx.toml`. Creating a shipped default file would
introduce a fresh surface (docs touch-up, loader branch
coverage, operator-discoverability decisions) that Phase 12
does not need and was not scoped for. The phase-level
property Task 3 pins is "roles actually work for real
product tools, not just for shell.exec" — and that property
is provable at the **agent layer** with
`FakeTool`/`VecPlanner`/`RecordingAudit`, the same harness
Phase 11 Task 4's allowlist tests use. Task 3 therefore ships
exactly one thing: **a cross-role regression test** at
`crates/aivyx-core/src/agent.rs` that drives the *same tool
registry* twice through the agent layer with two different
`with_tool_allowlist` configurations (`coder` = shell.exec,
`researcher` = web.fetch) and asserts:

1. The `coder` agent calling `web.fetch` is denied at the
   allowlist gate with scope `tool.allowlist:web.fetch`,
   emits no `ToolCall` audit event, and routes through
   `ToolOutcome::Denied`.
2. The `researcher` agent calling `shell.exec` is denied
   at the allowlist gate with scope
   `tool.allowlist:shell.exec`, symmetrically.
3. Each agent's *in-role* call (`coder` → `shell.exec`,
   `researcher` → `web.fetch`) succeeds and emits a
   normal `ToolCall` audit event.

The three-assertion shape gives cross-role coverage without
any config-file infrastructure. If a later phase ships a
default TOML file, the agent-layer regression stays
load-bearing — config parsing cares about what's *in* the
file; agent-layer regression cares about what happens *after*
the file is parsed.

**What this means for the draft bullets:**
- "Default `researcher` role's `tool_allowlist` gains
  `web.fetch`" — **N/A**, no default role to update.
- "Default `researcher` role gets one `web.fetch:url-prefix:`
  capability grant in the default `aivyx.toml`" — **N/A**,
  no default TOML.
- "Cross-role regression test: same physical TOML config
  produces two working agents" — **amended**: same physical
  **tool registry** produces two working agents via two
  distinct `with_tool_allowlist` calls. The phase-level
  property (roles actually work for product tools) is
  unchanged; only the test's level changes from
  binary-through-TOML to agent-through-fixture.
- "Possibly a small docs touch-up to the Phase 11 example
  config" — **N/A**, no example config.

**Task 4 / Task 5 implications:** Task 4's "default config
file" candidate, if anyone had one, is now dead. Task 5 exit
freeze proceeds as normal.

**Production-core byte-identity:** Task 3 is pure additions
to the `#[cfg(test)]` block of `agent.rs` — zero production
surface change, zero `lib.rs` touch. The byte-identity
streak (already broken in Tasks 1 and 2) is not further
broken here; `lib.rs` stays identical to its post-Task-2
state.

---

## Task 1 — shipped (2026-04-15)

**What landed.**

1. **`StreamEvent::ToolOutput { tool, tool_name, chunk }`
   variant** in `crates/aivyx-core/src/lib.rs`. Borrowed to
   match the existing variants (`&'a str` chunk, UTF-8 only).
   `tool` + `tool_name` carried for symmetry with
   `ToolCallStarted` / `ToolCallFinished` so renderers that
   eventually need to interleave chunks from concurrent tool
   calls can do so without a follow-up variant widening.
2. **Three exhaustive-match sites updated** — Local renderer
   (`crates/aivyx-channel/src/render.rs`), Telegram renderer
   (`crates/aivyx-telegram/src/telegram_channel.rs`), and the
   agent test recorder (`crates/aivyx-core/src/agent.rs`).
   The Tool trait itself is **not** widened: emission goes
   through the already-existing `ToolContext::channel.
   stream_event(StreamEvent<'_>)` seam — see the Task 1
   correction block above for why the draft's "new streaming
   sink parameter on `Tool::run`" plan was dropped.
3. **Rendering decisions.** Local renderer writes chunks
   verbatim between the existing start/finish markers (same
   arm shape as `StreamEvent::Text`) — no per-chunk header,
   no delimiter. Telegram renderer is a deliberate **no-op**:
   per-chunk Telegram rendering would need a per-tool-call
   accumulator on the channel struct, which is more surface
   than the Phase 11 trust-tier asymmetry pattern justifies.
   Telegram users see the Phase 11 shape unchanged — start
   marker, finish marker, `outcome_summary` one-liner — and
   the regression test `telegram_tool_output_chunks_are_
   dropped` pins the secret-chunks-never-leak invariant with
   three SECRET-marked chunks.
4. **Audit bridge: zero code change.** Audit entries are
   written by `AuditHook` method calls at specific turn-loop
   points, not by pattern-matching on `StreamEvent`. Task 1
   does not touch `aivyx-audit`, and the "exactly one
   `AuditTag::ToolCall` per tool call regardless of chunk
   count" invariant is pinned by a regression test
   (`five_chunks_produce_exactly_one_toolcall_audit`).
5. **`ScriptedStreamingTool` test subject** lives inside
   `#[cfg(test)]` in `agent.rs` next to `FakeTool`, driven by
   a `Vec<String>` of chunks that are emitted via the
   `ToolContext::channel.stream_event` seam during
   `execute()`. Schema is stored as an owned `Value` on the
   struct, matching `FakeTool`'s shape and avoiding any new
   `once_cell`-style static cache.

**Exit criteria — all met.**

- ✅ `StreamEvent::ToolOutput` variant lands with borrowed
  `tool_name` + `chunk` fields and a `tool: ToolId` for
  forensic symmetry.
- ✅ Three exhaustive-match sites extended; `cargo build`
  fails closed for any site the refactor misses.
- ✅ Local renderer streams chunks verbatim; Telegram
  renderer drops chunks silently; audit bridge emits
  exactly one `ToolCall` tag per tool call independent of
  chunk count. All three rules locked by regression tests.
- ✅ Q6 resolved early (variant shape, pre-implementation)
  and Q2 resolved-in-passing (cancellation plumbing was a
  no-op — `ToolContext.cancellation` already propagates
  naturally; no streaming-specific plumbing needed).
- ✅ `cargo test --workspace` green: **422 → 429 passed**,
  delta **+7** for Task 1 alone, above the draft's ≥+5
  target.
- ✅ `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- ✅ Zero new workspace dependencies.
- ⚠ **Production-core `lib.rs` byte-identity streak:
  broken** (27-line diff vs Phase 11 exit `16422e2`). This
  was expected — the new variant plus its doc comment is
  additive within D3's illustrative code block, not a
  contract change. Will re-baseline at the Phase 12 exit
  commit. Task 1 is the first break this phase; Task 2's
  break (another re-export) layered on top of this one.

**Deferred (recorded so the backlog doesn't silently grow).**

- **Per-chunk Telegram rendering.** Would need a per-tool-
  call `HashMap<ToolId, String>` accumulator on
  `TelegramChannel` plus an edit-message flush on finish.
  Not load-bearing; re-opens only if Telegram operators ask
  for it.
- **Binary (non-UTF-8) chunks.** The `&'a str` chunk shape
  restricts to UTF-8. A future `Tool::run` that needs to
  stream binary would either add a second variant
  (`ToolOutputBytes { chunk: &'a [u8] }`) or base64-wrap at
  the tool level. No phase currently needs this.

---

## Task 2 — shipped (2026-04-15)

**What landed.**

1. **`WebFetchTool`** under `crates/aivyx-core/src/tools/web_fetch.rs`
   — mirrors the `ShellExecTool` layout: `WebFetchToolConfig::new().build()`
   returns a ready-to-register tool carrying an `Arc<reqwest::Client>`
   configured with `redirect::Policy::none()` + 10-second
   connect timeout. Input schema is flat (`{url, timeout_ms?}`),
   no nested-validator dependency. Name is `web.fetch`; scope
   base is `net.fetch` (see the mid-implementation correction
   block above for why).
2. **GET-only, UTF-8 body, 10 MiB hard cap.** Body is streamed
   through `StreamEvent::ToolOutput` as UTF-8 chunks arrive
   (chunks that fail the per-chunk UTF-8 check are skipped
   from streaming but still included in the aggregated body
   the planner sees), and a size check before every
   `extend_from_slice` rejects oversize responses as
   `ToolOutcome::Failed` rather than truncating or OOMing.
3. **Capability-layer hardening (Q4 resolution).** The existing
   `QualifierKind::UrlPrefix` matcher at
   `aivyx-capability/src/lib.rs` was using raw
   `needed_q.starts_with(held_q)`, which admitted the classic
   hostile-suffix attack (held `https://example.com/` grants
   needed `https://example.com.evil.com/`). Replaced with a
   hand-rolled origin-aware matcher `url_prefix_grants` +
   `parse_scope_url` helper, covering:
   - (scheme, host, port) exact match with default-port
     normalization (80/443) and ASCII-case-insensitive host
     compare.
   - Path-prefix on component boundaries — trailing-slash
     equivalence (`/api` and `/api/` are the same directory)
     but rejecting non-boundary prefixes (`/users` does NOT
     grant `/users2`).
   - Query and fragment are stripped from the needed URL
     before matching.
   - Zero new workspace dependencies — the full parser is
     `&str` arithmetic with a couple of `split_once`s.
4. **Registration wiring.** New helper
   `build_web_fetch_for_channel(channel_kind) -> Result<Arc<dyn Tool>, String>`
   in `crates/aivyx-channel/src/bin/aivyx.rs`, returning the
   tool unconditionally for both `ChannelKind::Local` and
   `ChannelKind::Telegram`. Registered into the shared
   `tool_list` after `build_shell_exec_for_channel`. The
   binary's operator-held capability set gains an unqualified
   `net.fetch` scope so D4 Rule 2 grants all per-URL
   `net.fetch:<url>` requests by default on the Trusted CLI;
   ceiling intersection on Telegram narrows it via
   `CEILING_SEMITRUSTED` (which also holds unqualified
   `net.fetch`).
5. **Regression tests.** 22 new tests across two crates:
   - `aivyx-capability/src/lib.rs`: 9 URL-matcher regression
     tests — hostile-suffix hostname (×2), non-boundary path
     prefix, scheme mismatch, default-port normalization,
     case-insensitive host, trailing-slash equivalence,
     query-string ignored, malformed-denied.
   - `aivyx-core/src/tools/web_fetch.rs`: 12 tool tests —
     scope derivation (happy path + four deny-scope paths:
     missing URL, empty URL, `file://`, `ftp://`), capability
     integration (origin grants subpath, different-host
     denial, hostile-suffix denial — the tool-level
     regression for the same security property), schema
     shape, and three execute-path error tests (malformed
     URL, missing URL, unresolvable `.invalid` host).
   - `aivyx-channel/src/bin/aivyx.rs`: 2 binary gate tests —
     `channel_local_receives_web_fetch` and
     `channel_telegram_receives_web_fetch`, symmetric
     counterparts to the Phase 11 Task 3 shell.exec gate
     pins.

**Exit criteria — all met.**

- ✅ `web.fetch` registered for Trusted and SemiTrusted tiers,
  not for Untrusted/Kernel (which aren't wired in the binary
  anyway — the `ChannelKind` enum only covers `Local` and
  `Telegram`).
- ✅ Scope base correction recorded mid-implementation (tool
  name `web.fetch`, scope base `net.fetch`).
- ✅ Hostile-suffix regression test (`url_prefix_rejects_hostile_
  suffix_hostname` + `held_origin_does_not_grant_hostile_
  suffix_host`) asserts Q4 resolution is implemented
  correctly at both the capability-layer and tool-integration
  layers.
- ✅ Response body size limit (`MAX_BODY_BYTES = 10 MiB`)
  surfaces as `ToolOutcome::Failed` before exceeding memory.
- ✅ `cargo test --workspace` green: **451 passed, 0 failed**
  (429 → 451, delta **+22**, acceptance ≥+10).
- ✅ `cargo clippy --workspace --all-targets -- -D warnings`
  clean (needless-borrow regression on the strip_suffix
  `unwrap_or` spotted by clippy and fixed before landing).
- ✅ Zero new workspace dependencies. `reqwest`, `bytes`,
  `futures-util` were already in `[workspace.dependencies]`
  (via `aivyx-llm`) — Task 2 just promoted them to direct
  deps of `aivyx-core`.
- ⚠ **Production-core `lib.rs` byte-identity streak:
  broken** (47-line diff vs Phase 11 exit `16422e2`). The
  `pub use tools::web_fetch::...` re-export is the only
  change, and it was forced by the one-re-export-site
  convention. Will re-baseline at the Phase 12 exit commit.
  Task 1's identical break has already been recorded; this
  is the second break this phase, but the same "new tool
  touches the re-export surface" cause.
- ⚠ **Task 4 candidate changed.** The draft expected Task 4
  to be most likely "audit-payload widening for response
  headers." Task 2 settled that question by punting: the
  tool's return value is `{url, status, body}` with no
  headers, and no header propagation reaches the audit
  layer. So Task 4's most-likely candidate is now the URL
  canonicalization deep-dive (candidate #2 in the draft) —
  if indeed anything rolls up to Task 4 at all.

**Deferred to later phases (recorded here so the backlog
doesn't silently grow).**

- **Response headers in audit payload.** The Q3 pin said
  "audit-log-only, not model-visible." Task 2 ships the
  "not model-visible" half; the "audit-log-only" half is
  deferred because the existing audit payload has no header
  field and adding one spans `aivyx-audit` +
  `aivyx-core::ToolCallFinished` + at least one audit bridge
  — out of scope for a single task. Recorded as a Phase 12
  deferral and a backlog item for Phase 13+.
- **Non-GET verbs (POST/PUT/DELETE).** Pinned Q1 — deferred
  indefinitely.
- **Redirect following with per-hop scope re-check.** Pinned
  Q5 `Policy::none()` — deferred indefinitely.
- **Binary response bodies / non-UTF-8.** Task 2 fails loudly
  (`"response body from <url> is not valid UTF-8"`) rather
  than lossy-decoding. A later phase that needs binary can
  add a base64 encoding option to the return payload.

---

## Task 3 — shipped (2026-04-15)

**What landed.**

1. **Cross-role regression test at the agent layer.** Per the
   mid-implementation correction block above, Task 3 ships zero
   production surface change. Everything is additive inside
   `#[cfg(test)] mod tests` in `crates/aivyx-core/src/agent.rs`
   (+317 lines, no touches outside the test module).
2. **`run_single_tool_turn_with_allowlist` helper.** Small
   harness that wraps `ConcreteAgent::new` + `VecPlanner` +
   `FakeChannel` + `RecordingAudit` and returns
   `(TurnOutcome, Vec<AuditTag>)`. Lets each test scenario read
   one line instead of re-scaffolding an agent for every role.
3. **Test 1 — `cross_role_same_registry_different_allowlists_
   produce_asymmetric_access`.** Builds **one** `FakeTool`
   instance per tool (`shell.exec` and `web.fetch`) and
   `Arc::clone`s both into two separate
   `with_tool_allowlist` configurations (`coder` →
   `["shell.exec"]`, `researcher` → `["web.fetch"]`). Runs the
   four cross-product scenarios:
   - `coder` calling `web.fetch` → `ToolOutcome::Denied` with
     scope `tool.allowlist:web.fetch`, no `ToolCall` audit.
   - `researcher` calling `shell.exec` → `ToolOutcome::Denied`
     with scope `tool.allowlist:shell.exec`, no `ToolCall`
     audit.
   - `coder` calling `shell.exec` → success, `ToolCall` audit
     emitted.
   - `researcher` calling `web.fetch` → success, `ToolCall`
     audit emitted.
   The `Arc::clone` matters: it proves the asymmetry comes
   from the *allowlist view*, not from the two agents
   accidentally holding distinct tool registries.
4. **Test 2 — `cross_role_allowlist_gate_precedes_real_tool_
   execution`.** Stronger assertion: uses a `PanicOnExecute`
   fake whose `execute()` unconditionally panics. The test
   runs a role that is denied the tool and asserts the turn
   completes with `ToolOutcome::Denied` *without* the panic
   firing — proving dispatch short-circuits at the allowlist
   gate before ever reaching `execute`. This is the load-bearing
   invariant: a refactor that reordered the gate after execute
   would pass a plain-denied assertion but fail this one.

**Exit criteria — all met.**

- ✅ Cross-role regression test exists and passes, covering
  both "in-role succeeds" and "out-of-role denied" for two
  roles against the same shared tool registry.
- ✅ Allowlist-gate-before-execute invariant pinned by a
  `PanicOnExecute` fake (guards against silent reordering).
- ✅ Mid-implementation correction block recorded above
  (no shipped default TOML; no `examples/aivyx.toml` touch;
  scope change from binary-through-TOML to
  agent-through-fixture).
- ✅ `cargo test --workspace` green: **453 passed, 0 failed**
  (451 → 453, delta **+2**, acceptance ≥+2).
- ✅ `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- ✅ Zero new workspace dependencies. The test reuses the
  existing `FakeTool`/`VecPlanner`/`RecordingAudit`/
  `FakeChannel` harness already living in the same test
  module since Phase 11 Task 4.
- ✅ **Production-core `lib.rs` byte-identity: preserved**
  relative to post-Task-2 state. Task 3 touches only
  `agent.rs` (inside `#[cfg(test)]`) and `PHASE_12.md`; the
  existing break in `lib.rs` (from Tasks 1 and 2's
  `pub use` re-exports) is not widened.
- ⚠ **Task 4 is now pure slack.** Task 2's ship record
  already killed the "audit-payload widening" candidate,
  and Task 3's correction block killed the "default TOML
  file" candidate. The remaining Task 4 candidates from the
  draft are URL canonicalization deep-dive (slow, not
  load-bearing) and nothing else concrete. Most likely
  outcome: Task 4 is skipped outright and Phase 12 exits at
  Task 5 with three shipped tasks, same pattern as a couple
  of prior phases.

**Deferred to later phases (recorded here so the backlog
doesn't silently grow).**

- **Default role config file / `examples/aivyx.toml`.**
  Recorded in detail in the correction block above. If a
  future phase ships operator-facing configuration samples,
  the cross-role agent-layer regression stays load-bearing
  and the new binary-through-TOML test becomes additive, not
  a replacement.
- **Capability-side role scoping.** Task 3 pins the
  *allowlist* half of the "roles actually work" property; the
  *capability-set-per-role* half is already covered by the
  existing per-role `OperatorCapabilitySet`-through-config
  pathway from Phase 11 and needs no additional regression
  at this phase boundary.

---

## Task 4 — skipped (2026-04-15)

Task 4 was the draft's "working-session slot (contingent)"
— explicitly scoped as "a task that exists only if the
phase surfaces something worth a dedicated block, otherwise
it is skipped." By the time Task 3 shipped, every candidate
the draft listed had been closed:

- **Audit-payload widening for response headers.** Killed
  by Task 2's ship record: `web.fetch` returns
  `{url, status, body}` with no header propagation, so
  there is no header data reaching `ToolCallFinished` to
  widen. Deferred to a future phase per the Phase 12
  deferrals block below.
- **URL canonicalization deep-dive.** Subsumed by Task 2's
  `url_prefix_grants` / `parse_scope_url` work, which
  already handles (scheme, host, port) exact match with
  default-port normalization, ASCII case-insensitive host
  compare, trailing-slash equivalence, component-boundary
  path prefix, and query/fragment stripping. The nine
  regression tests in `aivyx-capability/src/lib.rs` cover
  every canonicalization rule the draft considered
  worth a working-session block.
- **Default `researcher` role TOML config file.** Killed
  by Task 3's correction block: there is no shipped
  default config file to update; the phase-level "roles
  actually work for real product tools" property is
  provable at the agent layer and already pinned by
  Task 3's cross-role regression.

With all three candidates closed, opening Task 4 would be
busywork. Task 4 is **skipped outright**, bringing Phase 12
to four shipped tasks (Task 1, Task 2, Task 3, Task 5) — the
same shape as the phases where the working-session slot
didn't fire.

---

## Task 5 — shipped (2026-04-15)

**What landed.**

Standard docs-only exit freeze, mirroring Phase 11 Task 5 /
Phase 10 Task 4 / Phase 9 Task 6.

1. **Ship records appended.** This commit is the first time
   Task 1's ship record lands in `PHASE_12.md` — Task 1 was
   committed without a ship record in place because the
   working order put ship records at freeze time rather than
   per-task. Tasks 2 and 3 already had ship records from
   their own commits. Task 4 gets a "skipped" record
   documenting why the working-session slot didn't fire.
2. **Decisions block.** `### Decisions made during Phase 12
   that aren't in DESIGN.md` records the Q1 and Q3 pins plus
   the Q2, Q4, Q5, Q6 task-level resolutions — the
   capability-layer hostile-suffix fix gets its own entry
   because it is a Phase 12 decision with load-bearing
   consequences for every future URL-scoped tool.
3. **Phase 12 deferrals block.** Enumerates what rolled up
   for Phase 13+ consideration: response headers in audit
   payload, non-GET verbs, redirect following with per-hop
   scope re-check, binary response bodies, per-chunk
   Telegram rendering, default role config file, forensic
   `ToolOutcome::NotInRole` variant (carried forward from
   Phase 11 unchanged — nothing in Phase 12 touched its
   case).
4. **Exit criteria (final)** checklist at the bottom of the
   document, mirroring Phase 11's shape exactly.
5. **`docs/README.md`** phase-status table flipped to
   Phase 12 **Frozen** (hash backfilled in a separate
   follow-up commit per the phase-discipline workflow).
6. **`docs/ROADMAP.md`** rolled: Phase 12 entry removed
   (it's now frozen here), Phase 13 entry scaffolded with
   a one-paragraph intent refined by what Phase 12
   learned.

**Exit criteria — all met.**

- ✅ Ship records for Tasks 1, 2, 3 (shipped), Task 4
  (skipped), and Task 5 (this commit) are all present
  above this line.
- ✅ Decisions block recorded.
- ✅ Deferrals block recorded.
- ✅ `cargo test --workspace` green at **453 tests** (422
  entry → 453 exit, delta **+31**, well above the ≥+10
  heuristic and above the ≥+17 draft target).
- ✅ `cargo clippy --workspace --all-targets -- -D
  warnings` clean at exit.
- ✅ Docs-only commit per the phase-discipline guardrail —
  no source edits, no `Cargo.toml` edits, no test edits in
  this commit.
- ✅ `docs/README.md` Phase 12 row updated to `Frozen` with
  hash placeholder for follow-up backfill.
- ✅ `docs/ROADMAP.md` rolled — Phase 12 entry removed,
  Phase 13 entry scaffolded.

---

### Decisions made during Phase 12 that aren't in DESIGN.md

- **Q1 — HTTP verbs:** **GET-only**, pinned at phase open.
  Write verbs (POST/PUT/PATCH/DELETE) are categorically out
  for Phase 12; HEAD was considered cheap-to-add but
  deliberately skipped to avoid "why not OPTIONS" scope
  creep. If a concrete use case for HEAD surfaces in a
  later phase, it lands as a task-local decision in that
  phase's journal. Resolved at phase open.
- **Q2 — Cancellation plumbing for streaming chunks:**
  **no-op.** `Tool::run` receives a `ToolContext` whose
  `cancellation` field already propagates from the turn
  loop's `CancellationToken`; streaming tools observe it
  the same way non-streaming tools do. No streaming-
  specific plumbing was needed. Resolved in Task 1 (by
  writing the code and discovering the plumbing was
  already complete).
- **Q3 — Response headers:** **audit-log-only**, pinned at
  phase open. The model sees `{url, status, body}` only.
  Task 2 punted the audit-log half (headers never reach
  `ToolCallFinished`'s payload at all in Phase 12); see
  the Phase 12 deferrals block for the forward-looking
  plan.
- **Q4 — URL-prefix scope matching:** **hand-rolled
  origin-aware matcher.** The existing capability-layer
  matcher was using raw `needed_q.starts_with(held_q)`,
  which admitted the hostile-suffix attack (held
  `https://example.com/` "matches" needed
  `https://example.com.evil.com/`). Replaced with
  `url_prefix_grants` + `parse_scope_url` in
  `aivyx-capability/src/lib.rs`: (scheme, host, port)
  exact match with default-port normalization and
  ASCII-case-insensitive host compare, path prefix on
  component boundaries with trailing-slash equivalence,
  query/fragment stripped from needed URL. **Zero new
  dependencies** — the matcher is 40-ish lines of `&str`
  arithmetic specialized for scope comparison, not a
  general-purpose URL parser. Resolved in Task 2.
- **Q4 bonus — the `net.fetch` scope base was buggy since
  Phase 1.** The `starts_with` bug had been shipping since
  the `QualifierKind::UrlPrefix` dispatch was first added;
  it had simply never been exercised by any tool with a
  URL-shaped scope (the other `UrlPrefix` callers were all
  test fixtures). Phase 12's first real `UrlPrefix`
  consumer surfaced the bug, the capability-layer fix is
  the load-bearing change, and the tool-level regression
  is the belt-and-suspenders layer.
- **Q5 — Redirect following:** **off.** `reqwest::redirect::
  Policy::none()` in `WebFetchToolConfig::build`. 3xx
  responses surface to the model as
  `{status: 302, body: ""}` (body empty because
  `Location` is a response header, not a body); the agent
  can choose to call `web.fetch` again on the redirect
  target, which will re-check the capability layer
  naturally. "On, but gated" was the alternative; it lost
  on simplicity plus putting redirect decisions in the
  model's context rather than hiding them in transport.
  Resolved in Task 2.
- **Q6 — `StreamEvent::ToolOutput` payload shape:**
  **`{ tool: ToolId, tool_name: &'a str, chunk: &'a str }`**,
  borrowed to match existing variants. UTF-8 only via
  `&str`; binary bodies are out of Phase 12 scope. `tool`
  + `tool_name` carried on every chunk for forensic
  symmetry with `ToolCallStarted` / `ToolCallFinished` and
  to future-proof against the day concurrent tool calls
  land. Resolved in Task 1.
- **Scope base reuse over scope base addition.** Task 2's
  draft assumed a new `web.fetch` scope base name; the
  correction block records why `net.fetch` (the base that
  was already in `KNOWN_BASES`) was reused instead. The
  load-bearing reason is **the tool name is independent of
  the scope base**: the tool answers "which tool was
  called," the scope answers "which capability was
  exercised," and the same scope base can front multiple
  tools that exercise the same capability family. Bakes
  in the D2 separation one level deeper than Phase 11's
  `shell.exec:cwd:<path>` shape showed.
- **Registration-time gate is reused from Phase 11 without
  modification.** `build_web_fetch_for_channel` in
  `aivyx-channel/src/bin/aivyx.rs` returns the tool
  unconditionally for both `ChannelKind::Local` and
  `ChannelKind::Telegram` — the pattern from Phase 11 Task
  3's `build_shell_exec_for_channel` generalizes cleanly
  to "channel-agnostic but still gated per-tool." The fact
  that `web.fetch` is actually channel-agnostic (both
  trust tiers include `net.fetch` in their ceiling) while
  `shell.exec` is not (only `TRUSTED` has it) did not
  require any change to the pattern shape, which is the
  Phase 11 handoff property the phase was supposed to
  validate.
- **Cross-role regression lives at the agent layer, not
  the TOML layer.** The draft assumed Task 3 would update
  a shipped default config file. None exists — `coder` /
  `researcher` are test fixtures only, not compiled-in
  defaults. The correction block in Task 3 records the
  scope change from binary-through-TOML to agent-through-
  fixture; the agent-layer regression stays load-bearing
  even if a later phase ships a default TOML (that phase
  would add binary-through-TOML on top, not replace the
  agent-layer test).
- **`PanicOnExecute` fake as an ordering invariant.** Task
  3's second test uses a tool whose `execute()`
  unconditionally panics, proving the allowlist gate
  short-circuits at the denied outcome **before** dispatch
  ever reaches `execute`. A plain "Denied" assertion would
  pass even if the gate fired after execute as long as the
  outer code routed to Denied; the panic makes the
  ordering mechanical. Pattern is worth reusing for any
  future gate that must run before a side-effectful stage.
- **Task 4 working-session slot empty is fine.** Phase 12
  is the first phase where the contingent Task 4 slot
  produced nothing worth doing. Recording that outcome
  explicitly in the skipped-record above is preferable to
  squeezing busywork into the slot.

### Phase 12 deferrals

Phase 12 entered carrying three rolling deferrals from
Phase 11 (streaming tool output, forensic `NotInRole`
variant, second regression channel). Task 1 consumed the
first; the other two carry forward unchanged.

**Rolling deferrals still open after Phase 12:**

- **Forensic `ToolOutcome::NotInRole` variant** — Phase 11
  Q1 deferral, untouched by Phase 12. Carries forward.
  Tagged: **Phase 11 Task 4, earliest plausible: whichever
  phase has a concrete forensic-tooling story that needs
  the `tool.allowlist:` scope distinction to be pattern-
  matchable on variant shape rather than scope base name.**
- **Second regression channel for the role primitive** —
  Phase 11 Q6 deferral. Untouched by Phase 12; reopens
  reactively only if a channel-seam bug surfaces that
  turn-loop tests miss.

**Net-new deferrals from Phase 12 itself:**

- **Response headers in audit payload (Q3 audit-log half).**
  The "model sees no headers" half shipped in Task 2; the
  "headers land in the audit payload" half is deferred.
  Adding it spans `aivyx-audit` +
  `aivyx-core::ToolCallFinished` + at least one audit
  bridge — a dedicated working-session task's worth of
  surface, out of scope for Phase 12. Tagged: **Task 2,
  earliest plausible: whichever phase has a concrete
  forensic story that wants response headers in the audit
  chain (a phase that ships a second-URL-scoped tool like
  `git.clone` where header inspection matters, or a phase
  that widens the audit schema for other reasons and can
  pick this up cheaply).**
- **Non-GET verbs (POST/PUT/PATCH/DELETE).** Q1 pinned
  GET-only for Phase 12. Tagged: **deferred
  indefinitely — reopens only when a concrete write-side
  use case surfaces.**
- **Redirect following with per-hop scope re-check.** Q5
  pinned `Policy::none()`. Tagged: **deferred
  indefinitely — reopens reactively if the fetch-
  redirect-fetch loop ergonomics become a real pain
  point in operator usage.**
- **Binary response bodies / non-UTF-8.** `web.fetch`
  currently fails loudly on non-UTF-8 bodies. Tagged:
  **deferred indefinitely — the first phase that needs
  binary fetches can add a base64-wrapping option or a
  second `ToolOutputBytes` stream variant.**
- **Per-chunk Telegram rendering.** Task 1 chose silent
  chunk drop on Telegram rather than a per-tool-call
  accumulator. Tagged: **Task 1, earliest plausible:
  reactive — reopens if Telegram operators ask for live
  in-progress tool output.**
- **Default role config file (`examples/aivyx.toml` or
  equivalent).** Recorded in Task 3's correction block.
  Tagged: **Task 3, earliest plausible: a later phase
  that ships operator-facing configuration samples as a
  first-class concern.**

**Backlog shape at Phase 12 exit:** two rolling items
inherited from Phase 11 + six net-new items from Phase
12. Total eight — up from Phase 11's exit total of three.
This is the first phase since Phase 6 where the foundation
backlog grew net-positive; every item is scoped,
originating-task-tagged, and reactive-trigger-tagged, so
the growth is "known deferrals" rather than "accumulated
debt."

### Exit criteria (final)

- [x] Task 1 shipped at `dc6ac22`: `StreamEvent::ToolOutput
      { tool, tool_name, chunk }` variant, three
      exhaustive-match sites extended, Local renderer
      verbatim / Telegram drop / audit one-tag-per-call
      invariants all regression-pinned, **+7 tests**.
- [x] Task 2 shipped at `ba9a724`: `WebFetchTool` at
      `TrustTier::Trusted` + `SemiTrusted` via
      `build_web_fetch_for_channel`, `net.fetch:<url>`
      scope shape, GET-only + 10 MiB UTF-8 body cap,
      capability-layer `url_prefix_grants` hardening
      that fixes a latent `starts_with` hostile-suffix
      bug shipping since Phase 1, **+22 tests**.
      Production-core byte-identity streak **broken
      again** here (47-line diff), layered on top of
      Task 1's break, re-baselined at this exit commit.
- [x] Task 3 shipped at `d31954f`: cross-role allowlist
      regression test at the agent layer — same
      `Arc::clone`-shared `FakeTool` registry driven by
      two `with_tool_allowlist` configurations, four
      cross-product scenarios, plus a `PanicOnExecute`
      ordering-invariant test. Pure `#[cfg(test)]`
      additions, zero production surface change, **+2
      tests**.
- [x] Task 4 **skipped**: working-session slot empty
      because all three draft candidates were closed by
      earlier tasks (audit-payload widening punted, URL
      canonicalization already covered in Task 2, default
      role config file doesn't exist). Skip recorded
      explicitly above rather than squeezing busywork
      into the slot.
- [x] Task 5 shipped at the commit this file freezes in:
      docs-only exit freeze — Task 1 ship record
      backfilled, Task 4 skip record recorded, Task 5
      ship record written, decisions block recorded,
      deferrals block recorded, `docs/README.md`
      flipped to Frozen, `docs/ROADMAP.md` rolled. Zero
      source edits.
- [x] `cargo test --workspace` green at **453 tests**
      (Phase 11 exit `16422e2`: 422 → Phase 12 exit:
      453, delta **+31**, well above the ≥+10 heuristic
      and above the draft's ≥+17 target).
- [x] `cargo clippy --workspace --all-targets -- -D
      warnings` clean at exit. Pre-commit hook held
      throughout.
- [x] **`DESIGN.md` byte-identical to `e0d6437`.**
      **Streak rolls to twelve consecutive phases.**
      Verified: `git diff e0d6437 HEAD -- docs/DESIGN.md
      | wc -l == 0`. No amendment file created during
      Phase 12.
- [x] **Production-core byte-identity streak: broken**
      — twice, both times in Task 1 (27-line diff adding
      `StreamEvent::ToolOutput`) and Task 2 (47-line
      cumulative diff adding the `WebFetchTool` /
      `WebFetchToolConfig` re-export on top of Task 1's
      variant). Both breaks are additive within D3's
      illustrative code block, no trait or outcome-
      variant contract change. Re-baselined at this
      exit commit. Task 3 left `lib.rs` byte-identical
      relative to its post-Task-2 state (all Task 3
      changes were inside `#[cfg(test)]` in `agent.rs`).
- [x] **Zero-new-dep streak: held.** Phase 12 added
      zero new workspace crates to `Cargo.lock`. The
      only `Cargo.toml` change is `crates/aivyx-core/
      Cargo.toml` promoting `reqwest`, `bytes`, and
      `futures-util` to **direct** deps — all three
      were already in `[workspace.dependencies]` via
      `aivyx-llm`, so no new `Cargo.lock` entries land.
      The zero-new-dep invariant is about new crates
      entering `Cargo.lock`, not about which crate
      declares an existing workspace dep directly.
- [x] Foundation backlog at Phase 12 exit: **eight
      items** (two rolling from Phase 11 + six net-new
      from Phase 12), enumerated and tagged in the
      Phase 12 deferrals block above. First phase since
      Phase 6 where the backlog grew net-positive;
      growth is known-deferral shape, not
      accumulated-debt shape.
- [x] `docs/README.md` phase-status table flipped to
      Phase 12 **Frozen**; commit hash backfilled in a
      separate follow-up commit per the
      `docs(phase-12): backfill` convention.
- [x] `docs/ROADMAP.md` rolled: Phase 12 entry removed,
      Phase 13 entry scaffolded with a one-paragraph
      intent refined by what Phase 12 learned about
      product-phase shape.
- [x] Task 1 ship record backfilled (committed without
      one during the working order, landed here).
- [x] Task 4 skip record explicit; the working-session
      slot being empty is recorded as a phase outcome,
      not silently dropped.
- [x] Q1 through Q6 all resolved and recorded in the
      "Decisions made during Phase 12 that aren't in
      DESIGN.md" block above. Q1 and Q3 resolved at
      phase open (pinned); Q2/Q4/Q5/Q6 resolved
      task-level (Task 1 for Q2 + Q6, Task 2 for Q4 +
      Q5).

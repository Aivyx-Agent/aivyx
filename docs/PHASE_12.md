# Phase 12 — `web.fetch` + streaming tool output (second product phase)

**Status:** Active (opened 2026-04-15). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–11 precedent.
**Predecessor:** [PHASE_11.md](PHASE_11.md) (exit commit `16422e2`,
hash backfill `8c12bc6`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
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

*(Task ship records land below as Phase 12 progresses. Follow
the Phase 11 shape: one `## Task N — shipped (YYYY-MM-DD)`
block per task, terminal exit-criteria checklist at the very
bottom of the file after Task 5 freezes.)*

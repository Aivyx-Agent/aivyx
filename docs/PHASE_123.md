# Phase 123 — Gmail Integration (Chapter F #1)

**Chapter F opener.** First phase under the external-
productivity-integrations chapter, opened after the audit's
#1 (Channel Activation Milestone) was deferred for the
twelfth time. Operator pressure picked a fresh thematic
axis over the verification-gap closeout.

## Chapter F context (this is the chapter opener)

After Chapters A (foundations), B (tooling, complete), C
(operator onboarding, paused), D (substrate breadth,
closed), and E (self-improvement loop deepening, #4 closed
by Phase 118), Chapter F opens a new thematic axis:
**Aivyx as a productivity assistant, not just a chat
surface.**

Productivity integrations live entirely in the third-party
tool tier per P10 + P11 + P12 — Aivyx core ships exactly
thirteen substrate tools forever; everything richer is a
third-party tool process the operator installs explicitly.
Chapter F is the chapter that validates that contract on
real third-party integrations: Gmail first (this phase),
Calendar / Drive / GitHub / etc next as the operator picks
at each exit.

Each Chapter F phase ships:
- One target integration as a separate binary crate.
- The auth substrate appropriate for that service (OAuth
  for Google services; PAT for GitHub; etc).
- Per-service capability scopes registered into the
  existing capability machinery (no new core types
  required — `Scope` already accommodates arbitrary
  bases).
- INSTALL.md walkthrough covering operator-side setup
  (auth-app registration, `aivyx auth` flow,
  `[[tool_process]]` registration, per-role scope
  granting).

The chapter expects to ship 3-5 integrations before
operator pressure redirects again. The "easy wins first"
discipline established for Chapter D applies: Gmail picked
first as the OAuth substrate establisher (Calendar and
Drive reuse the same Google OAuth pattern); GitHub /
others picked later as operator pressure dictates.

## Real-use signal driving this phase

After Phase 122 shipped, the audit's #1 (Channel Activation
Milestone) was deferred for the twelfth time. The deferral
count keeps growing; the operator explicitly chose a fresh
thematic chapter over closing the verification gap.

The pressure favoring external integrations:
- The eleven prior substrate/polish phases since the audit
  established that Aivyx-internal substrate is mature.
  Remaining substrate gaps are at the model layer (per
  Phase 122 finding) or at the operator-verification axis
  (Channel Activation Milestone — repeatedly deferred).
- Productivity integrations target a different user value
  axis: making Aivyx do operator-useful external work
  beyond conversation. This pulls on G6 (Local execution,
  privacy non-negotiable) — operator-provided OAuth keeps
  the privacy posture intact while opening a new value
  surface.
- The `[[tool_process]]` substrate (Phase 49, P12) has
  shipped but has no real third-party consumer yet. Phase
  123 validates the SDK contract on a real out-of-process
  integration. If the SDK is incomplete, Phase 123 finds
  out.

## P10 architectural lock-in

P10 caps substrate at thirteen tools forever and
**explicitly names email and calendar as third-party
territory**:

> "Every richer capability — browser, LSP, code search,
> **email, calendar**, anything domain-specific — is a
> third-party tool the operator installs explicitly."

Phase 123 therefore ships Gmail tools as a **separate
binary** (`aivyx-gmail`) wired via the existing
`[[tool_process]]` substrate per P11 + P12. This is not a
constraint to work around — it's the contract's intended
architecture for everything beyond bootstrap-essential
substrate. The decision validates two things at once:
1. The third-party SDK is actually usable for a non-
   trivial integration (OAuth, multiple tools, capability
   scopes).
2. The chapter pattern is reusable — Calendar / Drive /
   future Google integrations all reuse the OAuth
   substrate built here.

The substrate cost is the chapter cost, paid once in this
phase. Per-integration phases after Phase 123 are
substantially cheaper.

## Why this, why now

- **The substrate already exists.** P10 + P11 + P12 +
  Phase 49's `[[tool_process]]` substrate accommodate
  Gmail without contract amendment. The IPC envelope spec
  in `docs/TOOL_SDK.md` covers the full lifecycle. Phase
  123 is the first real consumer; if the SDK has gaps,
  this phase finds them.

- **Operator picked the full surface (Q2c) non-Recommended.**
  Read + draft + send all in one phase, rather than a
  conservative read-only first pass. Higher substrate
  value; higher review surface; risk acknowledged at
  sign-off. Phase 6 Q5 honest framing: if Q2c proves too
  much for one phase, the exit doc reports the partial
  shipping honestly rather than retconning the scope.

- **Operator-provided OAuth (Q1a Recommended) keeps the
  Phase 99 local-builds posture.** No Aivyx-published
  shared OAuth app; no shared GCP project for Aivyx to
  maintain; no Google verification limits to negotiate.
  Operator pastes their own client_id + client_secret;
  trust boundary is the operator's own GCP project.

- **Per-tool-process token file (Q3a re-asked) is the
  right substrate level for the first chapter F phase.**
  Each future Chapter F integration manages its own tokens
  similarly until shared-credential-vault pressure builds
  (likely Phase 125+). Avoids an upfront new-core-substrate
  detour for the chapter opener.

## Scope (Q-block sign-off)

- **Q1 — OAuth app registration model:** (a) **Operator-
  provided** (Recommended). Operator creates their own
  OAuth app in their own Google Cloud project; pastes
  `client_id` + `client_secret` into `aivyx.toml` (or the
  Gmail tool process's own config file). Aligns with G6
  + Phase 99 local-builds posture.

- **Q2 — Tool surface scope:** (c) **Read + draft + send**
  (non-Recommended). Full surface: `gmail.search`,
  `gmail.read`, `gmail.draft`, `gmail.send`. `gmail.send`
  gated to Trusted scope (Local only by default; mirrors
  `shell.exec` gating pattern). Honest scope acceptance:
  more substrate value than Q2a (read-only), more review
  surface. Phase 6 Q5 applies at exit.

- **Q3 (re-asked) — OAuth token storage:** (a) **Per-tool-
  process file** at `~/.aivyx/tool-processes/gmail/tokens.json`
  with 0600 perms (Recommended). Aligned with P6 single-
  operator OS-identity trust model; no new core substrate
  for the chapter opener.

The Q3 re-ask happened post-Q-block-sign-off when the P10
architectural constraint surfaced (the original Q3
encrypted-redb pick presupposed in-process tools; the P10
third-party constraint made redb structurally
unavailable). Documented honestly here rather than
retconned.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Phase 123 ships a new third-party tool process
  consuming the existing SDK contract; the SDK contract
  itself doesn't move. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to fourteen** (was 13
  after Phase 122).

- **PRODUCT.md** — **Will hold.** P10 + P11 + P12 + G6
  already cover this exact case (P10 explicitly names
  email as third-party territory; P11 + P12 cover the
  SDK + IPC contract; G6 covers operator-controlled
  credential posture). No new product commitment
  needed. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to fourteen** (was 13).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Will hold.** Phase 123 work lives in a new
  `aivyx-gmail` crate (separate binary; tool process).
  The existing `Tool` trait, `Scope`, `ToolContext`, etc
  are consumed unchanged. Hash at entry:
  `b420405bf9a5576ecb10f6ea04a965f7ad3a1a92ae9bd8f4abf46e22ef4d3c16`.
  Prediction: streak **extends to four** (was 3 after
  Phase 122). Honest 90/10 hold — the only risk is if
  Phase 123 surfaces a gap in the SDK that requires a
  trait extension; the existing substrate is mature
  enough that this risk is low.

- **New workspace deps** — At most 1 anticipated. Gmail
  needs HTTP client (`reqwest` already in workspace via
  `aivyx-llm`), JSON parsing (`serde_json` already
  workspace-wide), base64 encoding for MIME bodies
  (likely needs `base64` crate if not already present).
  OAuth flow will be hand-rolled minimal helper (avoids
  the `oauth2` crate's heavy dep tree).

- **Test count** — Substrate-heavy (OAuth flow + 4
  tools). Prediction: **+30 to +55**. Per-tool tests
  (schema validation, request-shape capture against
  mocked Gmail API), OAuth flow tests (auth-code
  exchange, refresh, token-file round-trip), CLI
  subcommand tests, integration tests against a mock
  Gmail server (httpbin-style or wiremock-rs if it's
  acceptable as a dev-dep).

## Tasks

Eight sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_123.md` + `docs/ROADMAP.md` Chapter F section
+ Phase 123 entry + `docs/README.md` status row. Documents
the P10 architectural lock-in surfaced post-Q-block
sign-off and the Q3 re-ask.

### Task 2 — `aivyx-gmail` crate + OAuth substrate

New `aivyx-gmail` binary crate (separate process). Ships:
- OAuth client config (operator-supplied `client_id` +
  `client_secret` via env vars or a small config file
  under `~/.aivyx/tool-processes/gmail/config.toml`).
- Auth-code exchange flow (POST to Google's token
  endpoint).
- Refresh-token flow (auto-refresh when access token
  near expiry; ~5 min before).
- Per-tool-process token file storage:
  `~/.aivyx/tool-processes/gmail/tokens.json` with
  0600 perms; atomic write-then-rename for crash safety.
- Tests: token storage round-trip; refresh-flow against
  mocked token endpoint; expiry-detection.

### Task 3 — `aivyx-gmail auth` CLI subcommand

Operator-facing first-time setup. The `aivyx-gmail` binary
exposes:
- `aivyx-gmail auth init` — opens browser to Google's
  OAuth consent screen, starts local-loopback HTTP
  server on `127.0.0.1:<port>` to capture the redirect,
  exchanges the captured code for tokens, writes to the
  token file.
- `aivyx-gmail auth status` — prints token health
  (granted scopes, expiry, refresh-available?).
- `aivyx-gmail auth revoke` — calls Google's token
  revoke endpoint and clears the local file.
- Tests: CLI parser tests; status output formatting;
  revoke flow.

### Task 4 — `gmail.search` tool

First operator-facing Gmail tool. Implements:
- Input schema: `{q: string, max_results: u32 (default 25,
  max 100), include_spam_trash: bool (default false)}`.
- Calls Gmail's `users.messages.list` with the operator-
  supplied query; iterates pages up to `max_results`.
- Returns: `{messages: [{id, thread_id, snippet,
  internal_date_ms, label_ids}]}`.
- Capability scope: `email.read`.
- Tests: schema validation; Gmail-query DSL pass-through;
  pagination cap; mock-server integration.

### Task 5 — `gmail.read` tool

One full message by ID. Implements:
- Input schema: `{id: string}`.
- Calls Gmail's `users.messages.get` with `format=full`.
- Returns: `{id, thread_id, headers: {...}, body_text,
  body_html, attachments: [{filename, mime_type,
  size_bytes, attachment_id}]}`.
- Attachment downloads NOT included in this phase
  (`attachment_id` returned for a future-phase
  `gmail.attachment.read` if needed).
- Capability scope: `email.read` (same as search).
- Tests: schema validation; MIME-multipart parsing; HTML/
  text body extraction; mock-server integration.

### Task 6 — `gmail.draft` tool

Safe write surface. Implements:
- Input schema: `{to: string, subject: string,
  body_text: string, in_reply_to_message_id: Option<string>}`.
  `in_reply_to_message_id` populates the `In-Reply-To` +
  `References` headers + matches `threadId` for
  conversation threading.
- Constructs RFC 5322 MIME message; base64url-encodes;
  calls Gmail's `users.drafts.create`.
- Returns: `{draft_id, message_id}`.
- Drafts require operator-initiated send via Gmail UI —
  no message actually leaves until the operator clicks
  send. This is the safe-write surface.
- Capability scope: `email.write`.
- Tests: MIME construction; thread-reply header set;
  schema validation; mock-server integration.

### Task 7 — `gmail.send` tool (Trusted-gated)

Direct send. Implements:
- Input schema: same as `gmail.draft` plus optional
  `from: string` for send-as aliases.
- Constructs RFC 5322 MIME; base64url-encodes; calls
  Gmail's `users.messages.send`.
- Capability scope: `email.send`. Registered Local /
  Trusted only by default — mirrors `shell.exec`
  gating pattern. Operators must explicitly grant
  `email.send` in a role's `capability_scopes` list to
  enable; agent-initiated sends from Telegram / Discord
  / Slack channels denied at the capability layer.
- Tests: capability denial on non-Trusted channels;
  successful send on Local; schema validation; mock-
  server integration; thread-reply via `in_reply_to`.

### Task 8 — INSTALL.md sweep + exit + hash backfill

Operator-facing documentation:
- New `## External productivity integrations (Chapter F)`
  section in INSTALL.md framing the chapter.
- Sub-section `### Gmail (Phase 123)` with full
  walkthrough: GCP OAuth app creation, `aivyx-gmail
  auth init` flow, `[[tool_process]]` TOML registration,
  per-role `email.read` / `email.write` / `email.send`
  scope granting examples, troubleshooting (token
  refresh failures, scope-denied errors, etc).
- PHASE_123.md exit doc: prediction-vs-reality, test
  count delta, SDK-contract validation findings (any
  gaps surfaced).
- ROADMAP.md rotation (Phase 123 → Frozen; Chapter F
  remains open).
- README.md status row → Frozen with exit-commit hash
  backfilled in standard second-step commit.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — OAuth app registration model:** (a) **Operator-
  provided** (Recommended).
- **Q2 — Tool surface scope:** (c) **Read + draft + send**
  (non-Recommended). Full surface in one phase; risk
  acknowledged at sign-off.
- **Q3 (re-asked) — OAuth token storage:** (a) **Per-tool-
  process file** (Recommended after re-ask under P10
  third-party constraint).

## Exit criteria

- [ ] `docs/PHASE_123.md` + ROADMAP Chapter F + Phase 123
  entry + `docs/README.md` status row — Task 1.
- [ ] `aivyx-gmail` crate skeleton + OAuth substrate —
  Task 2.
- [ ] `aivyx-gmail auth` CLI subcommand — Task 3.
- [ ] `gmail.search` tool — Task 4.
- [ ] `gmail.read` tool — Task 5.
- [ ] `gmail.draft` tool — Task 6.
- [ ] `gmail.send` tool (Trusted-gated) — Task 7.
- [ ] INSTALL.md sweep + exit — Task 8.
- [ ] Q1 / Q2 / Q3 (re-asked) resolved with operator
  sign-off pre-Task 2 (Q1a + Q2c non-Recommended + Q3a
  re-asked).
- [ ] DESIGN.md streak — predicted HOLD (streak → 14).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 14).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (streak → 4), honest 90/10 hold.
- [ ] At most 1 new workspace dependency (likely
  `base64` if not already present).
- [ ] Test count delta within `+30` to `+55`.
- [ ] Zero clippy warnings.
- [ ] **SDK-contract validation finding** in exit doc:
  any gaps the third-party-tool-process consumer
  surfaced in the existing SDK substrate. This is a
  load-bearing Phase 123 deliverable — if the SDK has
  gaps, future Chapter F phases need to know.

## Honest scope risks at sign-off

- **Q2c full-surface risk.** Read + draft + send in one
  phase is more review surface than Q2a (read-only).
  Phase 6 Q5 applies at exit: if the phase ends up
  shipping read + draft only (with send punted to a
  follow-up), the exit doc reports that honestly.

- **OAuth complexity may bite.** Operator-provided OAuth
  requires the operator to register an app in Google
  Cloud Console, configure consent screen, enable Gmail
  API, etc. INSTALL.md walkthrough is load-bearing;
  if the walkthrough has gaps, the phase fails on the
  operator-onboarding surface. Substrate work doesn't
  fix bad docs.

- **SDK contract may have gaps.** Phase 123 is the first
  real third-party tool process consumer. Gaps will
  surface — capability scope passing, audit-event
  shape, cancellation handling, error-envelope
  ergonomics, registration discovery. The exit doc's
  SDK-validation finding documents what (if anything)
  the SDK needs in a follow-up phase.

- **Google's OAuth verification process.** Operator-
  provided OAuth apps in "Testing" mode work only for
  pre-listed accounts (up to 100). For production-use,
  operators must publish their app and may need
  Google's app verification (depending on scopes
  requested). The Gmail scopes used (`gmail.readonly`,
  `gmail.compose`, `gmail.send`) include
  **sensitive scopes** that may trigger verification.
  INSTALL.md must surface this clearly.

## Direction after Phase 123

Chapter F is open and ongoing. After Phase 123, candidates
for Phase 124:
1. **Google Calendar** (Chapter F #2) — reuses the OAuth
   substrate established here; smaller surface (events
   list / create / update). Natural easy-wins-first
   pick if Phase 123 ships cleanly.
2. **Google Drive** (Chapter F #2) — reuses OAuth;
   substantial new surface (file types, format
   conversions, large-file transfer). Higher value but
   higher cost.
3. **GitHub (PAT auth)** (Chapter F #2) — different
   auth substrate (PAT/token, not OAuth); pulls the
   chapter into the second-auth-pattern territory
   sooner.
4. **Shared credential vault substrate** — extract the
   per-tool-process token file pattern into a shared
   core substrate (KeyDomain expansion, IPC credential
   API). Only worth doing if Phase 123 + the next
   integration surface enough pressure.
5. **Channel Activation Milestone** — twelfth-in-a-row
   deferral as of Phase 123 open; thirteenth if
   skipped at Phase 124 too. Audit's #1 unchanged.
6. **Operator-pressure-driven new direction**.

Phase-by-phase decision at Phase 123 exit, sharpened by
the SDK-contract validation finding from Task 8.

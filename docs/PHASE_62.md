# Phase 62 — Agent-Initiated Outbound Notifications

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Give the agent a `notify.send` infrastructure tool that pushes a
message to an operator-configured target. Two backends in this
phase: **Telegram** (via the existing `aivyx-telegram` bot
client's `sendMessage` path) and **generic webhook** (HTTP POST
with a small JSON body — covers ntfy.sh, Pushover, IFTTT,
custom endpoints).

Transforms Aivyx from purely reactive ("I talk to it") to
proactive ("it can wake my phone"). Opens the **Reach Milestone**
post-Distribution. Agent-driven only — the agent explicitly
calls `notify.send` when relevant. Trigger-config sugar
(automatic notify when a schedule/webhook/file-watcher fires) is
deferred to a later phase.

## Why now

1. **Codebase-review finding.** After Phase 61 closed
   Distribution's first phase, the next-largest adoption-shape
   gap is reach. Today's Aivyx responds; it doesn't *initiate*.
   Schedules/webhooks/file-watchers fire turns but the output
   stops at the audit log — there's no path from "agent has
   something to say" to "operator's phone buzzes." Closing this
   is the inflection point between "thing I talk to" and "thing
   that talks to me."
2. **Substrate already in place.** The Telegram bot client
   ships in tree (Phase 8) and already does outbound — the bot
   replies to operator messages every turn. Lifting that send
   path for use by a notification dispatcher is a small
   refactor, not new architecture. The webhook outbound path
   uses `reqwest`, already a workspace dep.
3. **No contract reshape.** Phase 62 sign-off chose the
   infrastructure-tool path over a P10 amendment. `notify.send`
   slots alongside `mission.create`, `schedule.create`,
   `reflection.propose` — channel-layer tools, not substrate.
   No DESIGN.md or PRODUCT.md touch.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 62 ships an infrastructure
  tool + capability scope + config surface + two dispatcher
  implementations. No D-deliverable reshape. Prediction: streak
  **extends to nine** consecutive phases (currently at 8 after
  Phase 61).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits, no
  Delivery Status refresh (P1–P14 stay Fully Delivered). The
  "Reach Milestone" entry lands in `docs/PRODUCT_ROADMAP.md`,
  not in the contract document. Prediction: streak **extends to
  two** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Every Phase 62 surface lives in `aivyx-channel` (dispatcher,
  tool, webhook impl, system-prompt update), `aivyx-capability`
  (scope base + ceiling), `aivyx-config` (config struct), and
  `aivyx-telegram` (exposed outbound function). No path
  touches `aivyx-core`. Prediction: streak **extends to ten**
  consecutive phases (new record, beating Phase 61's 9).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_62.md scaffold

This file. Update `docs/README.md` to show Phase 62 as Open.
Q-block resolutions committed before Task 2.

### Task 2 — `notify.send` capability scope

`aivyx-capability` gains a new `notify.send` base in
`KNOWN_BASES`, included in `CEILING_TRUSTED`. A new
`QualifierKind::TargetName` (or reuse of the existing target-
name qualifier shape from `role.switch`) lets roles narrow to
specific targets via `notify.send:<target_name>`. The
unqualified `notify.send` is the wildcard. Per **Q1** at
sign-off, single-level qualifier matches the `role.switch:<name>`
precedent.

### Task 3 — `[[notify_target]]` TOML config surface

`aivyx-config` gains `NotifyTargetConfig` with `name`, `kind`
(enum: `Telegram | Webhook`), and kind-specific fields:

- `Telegram`: `chat_id: String` (the operator's Telegram chat
  the bot is authorized to message; reused from the existing
  `[telegram]` config).
- `Webhook`: `url: String` (the endpoint to POST to).

Load-time validation:

- Names unique across all `[[notify_target]]` entries.
- Telegram kind requires `chat_id`; webhook kind requires `url`.
- Other kind-specific keys rejected as unknown.

### Task 4 — `NotifyDispatcher` + runtime types

New file `crates/aivyx-channel/src/notify_dispatcher.rs`. A
trait `NotifyBackend` with one `async fn send(&self, message:
&str, subject: Option<&str>) -> Result<(), NotifyError>`. The
dispatcher is a name-keyed registry of `Box<dyn NotifyBackend>`
constructed at daemon startup from the loaded
`NotifyTargetConfig` entries.

`NotifyError` is a small enum: `Transport(String)`,
`Auth(String)`, `Rejected(u16)`, `Timeout`. Used by Task 7's
tool to return `success=false` with structured failure reason
in the output.

### Task 5 — Telegram outbound backend

`aivyx-telegram` exposes a public `send_message(chat_id: &str,
text: &str, parse_mode: Option<ParseMode>) -> Result<(),
SendError>` function reusing the existing bot client's
`sendMessage` path. The `NotifyBackend` impl for Telegram wraps
this with the target's `chat_id` baked in and `subject` (if
provided) prepended as bold Markdown.

### Task 6 — Generic webhook backend

`NotifyBackend` impl for the webhook kind. `reqwest::Client`
POST to the target's URL with:

- Body: JSON `{source: "aivyx", target, subject, message,
  timestamp}` per **Q5** at sign-off.
- Headers: `Content-Type: application/json`.
- Timeout: 5 seconds.

HTTP 2xx → `Ok(())`. 4xx → `NotifyError::Rejected(status)`. 5xx
→ `NotifyError::Transport(status)`. Timeout → `NotifyError::Timeout`.

### Task 7 — `NotifySendTool`

New `crates/aivyx-channel/src/notify_tool.rs`. `Tool` impl with
the `OnceLock` factory pattern (matches `MissionCreateTool`,
`ScheduleCreateTool`). Input schema per **Q3**:

```json
{
  "type": "object",
  "properties": {
    "target":  { "type": "string", "description": "Name of a configured notify_target" },
    "message": { "type": "string", "description": "Notification body" },
    "subject": { "type": "string", "description": "Optional short subject line" }
  },
  "required": ["target", "message"]
}
```

Execute path:

1. Look up `target` in the dispatcher; if not configured, return
   `ToolOutcome::Failed` with "unknown notification target".
2. Call the backend's `send`. On success, return
   `ToolOutcome::Completed` with output `{success: true,
   target, delivered_at: <RFC 3339>}`.
3. On `NotifyError`, return `ToolOutcome::Completed` with
   output `{success: false, target, error_kind: <variant>,
   error_message: <string>}` per **Q4**. The tool *call*
   succeeded; the *delivery* didn't. Agent reads the result
   and decides retry/escalate/give-up.

### Task 8 — Worked example + system-prompt surfacing

- `examples/aivyx.toml` gains a `[[notify_target]]` Telegram
  block and a `[[notify_target]]` webhook block, each with
  rationale comments.
- A `coder` or `default` role example gains `notify.send` in
  `tool_allowlist` and `capability_scopes` to demonstrate the
  full plumbing.
- `assemble_session_prompt` in `aivyx-channel` extends with a
  small "## Notification targets" block, conditional on the
  current role having `notify.send` scope AND at least one
  target configured. Format: bullet list of `<name> (<kind>) —
  available`. This is how the agent learns *what* it can
  reach; *when* to use it remains the operator's
  Profile/system-prompt concern.

### Task 9 — Exit commit

- `ROADMAP.md` Phase 62 frozen entry.
- `docs/PRODUCT_ROADMAP.md` new "Reach Milestone" section with
  Phase 62 listed as phase 1 of N (notifications now; future
  sub-phases for trigger-config sugar, Web UI desktop notify,
  email SMTP, additional channel adapters).
- `docs/README.md` status table flipped to Frozen with backfill
  per project convention.
- Prediction-vs-reality block filled.
- Exit-criteria block completed.

## Open questions

**Q1 — Scope qualifier shape?**

  - **(a)** `notify.send:<target_name>` single-level qualifier
    (matches `role.switch:<name>`, `mcp.call:<server>:<tool>`).
  - **(b)** `notify.send:<kind>:<target_name>` two-level
    qualifier (kind first, then name).

  **Recommendation: (a).** Single-level is the standard for
  "external-resource by name." Two-level adds redundancy since
  kind is already declared in the target's config — the
  capability layer doesn't need to re-encode it.

**Q2 — Capability tier inclusion?**

  - **(a)** `CEILING_TRUSTED` only. SemiTrusted roles cannot
    notify.
  - **(b)** `CEILING_TRUSTED` + `CEILING_SEMITRUSTED`. SemiTrusted
    roles can notify if their role config opts in.

  **Recommendation: (a).** Phase 62 starts conservative;
  relaxing later is easy. A SemiTrusted role notifying without
  an extra friction step is a meaningful capability extension —
  notifications can leak data across trust boundaries. Phase
  63 can relax this if real use surfaces.

**Q3 — Tool input schema fields?**

  - **(a)** `{ target, message }` minimum.
  - **(b)** `{ target, message, subject? }` with optional
    short subject.
  - **(c)** `{ target, message, subject?, level? }` with
    severity level (info/warn/critical).

  **Recommendation: (b).** Subject is meaningful for webhooks
  (ntfy.sh, Pushover both have title fields); for Telegram it
  prepends as bold Markdown. Severity is design overreach for
  v1 — operators who care can encode it in the message body.

**Q4 — Notify failure semantics?**

  - **(a)** Tool returns `Completed` with `success=false` in
    output on delivery failure. Agent reads the result and
    decides retry/escalate/give-up.
  - **(b)** Tool returns `ToolOutcome::Failed` on delivery
    failure. Agent sees this as an error.
  - **(c)** Fire-and-forget — tool returns "Sent" regardless
    of actual delivery. Audit log carries the failure for
    forensic review.

  **Recommendation: (a).** Notifications are user-facing; the
  agent should know whether delivery worked AND have
  structured information about *why* it didn't (transport vs
  auth vs rejected). `Failed` is the wrong outcome shape —
  the tool *call* succeeded; the *delivery* failed. Fire-and-
  forget loses the actionable result data.

**Q5 — Webhook payload shape and headers?**

  - **(a)** Generic JSON body `{source, target, subject?,
    message, timestamp}` + `Content-Type: application/json`.
    Operator's webhook endpoint adapts.
  - **(b)** Configurable template per target — operator
    declares `body_template = "..."` and `headers = {...}`
    in target config. Maximum flexibility, real config
    complexity.
  - **(c)** Provider-specific helpers — `kind = "ntfy"`,
    `kind = "pushover"`, etc., each with built-in payload
    formatting.

  **Recommendation: (a).** Generic JSON covers ntfy.sh (which
  accepts JSON), Pushover (with the right URL), IFTTT, and
  custom endpoints. (b) and (c) are forward extensions if
  adoption shape demands. Slack-flavored payload (`{text:
  ...}`) is a real mismatch — defer as its own `kind` if
  pressure surfaces.

## Deferrals

**Rolling deferrals carried into Phase 62:**

- v0.1.0 publication (Phase 61 Task 7) — held under VPS-first
  posture.
- Identity export/import (Phase 60) — multi-device portability.

**Phase 62 deferrals (recorded at exit):**

- **System-prompt `## Notification targets` block.** The Task
  8 plan called for `assemble_session_prompt` to enumerate
  reachable targets so the agent learns them from the prompt
  itself. Implementation revealed six `assemble_session_prompt`
  call sites in the binary plus a handful in
  `profile_prompt.rs` tests; adding the parameter mechanically
  is straightforward but the actual *wiring* of "which role
  reaches which targets" needs per-call-site capability checks
  that are scope-creepy for one phase. Deferred. The tool is
  fully functional without it: the agent learns `target` is a
  configured-target-name from the input schema, and the
  operator's role `system_prompt` or `[profile]
  communication_style` can name specific targets when relevant.
  A future phase can lift the prompt enumeration when adoption
  shape demands.
- **Web UI desktop notification kind** (needs WebPush or
  aggressive polling).
- **Email SMTP outbound kind** (new dep like `lettre`,
  separate phase).
- **OS-level notifications** (per-platform code; marginal
  value vs webhook-out to ntfy.sh).
- **"Default target" sugar** (`notify.send {message: "..."}`
  without target arg).
- **Per-target rate limits** — could surface if abusive
  agents send too frequently.
- **Trigger-config `notify_target` sugar** (the deferred Q3
  alternative — `[[schedule]]` / `[[webhook]]` / `[[file_watch]]`
  configs gaining a `notify_target = "..."` field for
  automatic post-turn push without agent involvement).
- **Notification templates / per-target body formatting**.
- **Slack-flavored webhook payload** — mismatch between the
  generic `{message}` shape and Slack incoming webhooks'
  `{text}`. Lands as a separate `kind` if real Slack use
  surfaces.
- **Shared Telegram transport between channel-mode and
  notify-mode.** Phase 62 builds a separate `ReqwestTransport`
  for notify when telegram targets exist. Both transports use
  the same bot token and talk to the same API; the duplication
  is wasteful but correct. Sharing would require lifting the
  transport construction earlier in the binary's session-build
  path.

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak **extends to nine**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Phase 62 shipped entirely as infrastructure under existing
  D-deliverables — no contract reshape.

- **PRODUCT.md** — Predicted: streak **extends to two**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  No commitment-text edits, no Delivery Status refresh.
  P1–P14 stay Fully Delivered. The new "Reach Milestone" entry
  landed in `docs/PRODUCT_ROADMAP.md`, not in the contract
  document.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to ten** (new record). **Reality: correct.**
  Hash unchanged at entry and exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Every Phase 62 surface routed through `aivyx-capability`
  (notify.send scope), `aivyx-config` (`[[notify_target]]`
  parsing), `aivyx-channel` (dispatcher + backends + tool +
  daemon wiring), and `aivyx-telegram` (existing transport
  consumed unchanged). Ten consecutive phases beats the prior
  Phase 61 record of nine.

- **Test count** — Predicted: positive (~+20). **Reality:
  +65** (1074 → 1139), more than triple the prediction. The
  underestimate came from the per-task test density:
    * Task 2 — scope (6 tests)
    * Task 3 — config parsing (8 tests)
    * Task 4 — dispatcher + build helper (7 + 6 tests)
    * Task 5 — Telegram backend (15 tests)
    * Task 6 — Webhook backend (12 tests)
    * Task 7 — tool integration (11 tests)
  The prediction was reasonable for one or two of these
  surfaces; six concurrent surfaces each carrying ~10 tests
  added up.

- **New workspace deps** — Predicted: zero. **Reality:
  correct.** No new Rust crates added. The new code uses
  `reqwest`, `serde`, `serde_json`, `chrono`, `async-trait`,
  `thiserror`, `secrecy`, and `aivyx-telegram` — all already
  in tree.

## Exit criteria

## Exit criteria

- [x] `notify.send` capability scope wired into KNOWN_BASES
  and CEILING_TRUSTED with `<target_name>` qualifier — Task 2,
  commit `17c7e7d`.
- [x] `[[notify_target]]` TOML config surface with load-time
  validation (unique names, kind-required fields) — Task 3,
  commit `cc0fb2b`.
- [x] `NotifyDispatcher` + `NotifyBackend` trait + name-keyed
  registry — Task 4, commit `2559234`.
- [x] Telegram outbound backend wraps the existing bot
  client's `sendMessage` — Task 5, commit `74ca8fd`.
- [x] Generic webhook outbound backend POSTs the Q5(a) JSON
  body with 5s timeout — Task 6, commit `9abb5d1`.
- [x] `NotifySendTool` `Tool` impl with OnceLock factory,
  `{target, message, subject?}` schema, Q4(a) failure
  semantics — Task 7, commit `7ee0651`.
- [x] Worked example with both kinds in `examples/aivyx.toml`;
  daemon wiring builds dispatcher + registers tool — Task 8,
  commit `630136d`. **Scope adjustment:** the
  `assemble_session_prompt` extension originally planned for
  Task 8 deferred to a follow-up phase (see Deferrals).
- [x] ROADMAP.md + PRODUCT_ROADMAP.md (new Reach Milestone) +
  docs/README.md refreshed — Task 9 (this commit).
- [x] All five Q-block questions resolved with operator
  sign-off pre-Task 2 (Q1(a) single-level qualifier, Q2(a)
  Trusted only, Q3(b) `{target, message, subject?}`, Q4(a)
  Completed-with-success-false, Q5(a) generic JSON body).
- [x] DESIGN.md streak extends to nine.
- [x] PRODUCT.md streak extends to two.
- [x] Production-core streak extends to ten (new record).
- [x] Test count delta: +65 (1074 → 1139).
- [x] Zero clippy warnings.
- [x] Prediction-vs-reality block filled.

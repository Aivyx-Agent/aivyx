# Config-write surface area (POLISH_WAVES.md sub-project 7) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 7 bundles 6 findings from `VITRINE.md`
under one framing: "the first time Aivyx handles real credentials
through a web form rather than hand-edited TOML." Grounded directly
against the current code (not the tracking doc's prose) before any
design decision:

- **Autonomy-tier confirm dialog** (item F) — **already fully shipped**
  (Chapter Reins: `crates/aivyx-web/src/main.rs`'s `SettingsPanel`, a
  confirm-first modal for autonomy-granting levels, enforced
  server-side via `set_autonomy_query(level, confirm: bool)`). Nothing
  to build; dropped from this design.
- **MCP tool-level health signal** (item B) — already correctly noted
  in the doc as moved to sub-project 8. Not in scope here.
- **Schedules screen** (item D) — **partially shipped**. Studio
  already has full schedule creation (Chapter Chime: a cron-builder
  form, `FrontendMessage::CreateSchedule`), and `UpdateSchedule`/
  `DeleteSchedule` are already wired in the UI
  (`crates/aivyx-web/src/main.rs:2007`/`2024`). What V09_PLAN row 8
  actually still needs is much smaller than "build a screen": found by
  reading `crates/aivyx-channel/src/schedule_tool.rs` directly,
  `schedule.create`'s tool already blocks agent self-scheduling via
  `GrowthAdoption::None` (a knob `AutonomyLevel::expand()` derives from
  the `[autonomy]` dial — `crates/aivyx-config/src/autonomy.rs`), plus
  a frequency floor and an agent-schedule count cap. `schedule.update`
  and `schedule.delete`'s `execute()` methods have **no equivalent
  check at all** — confirmed by reading both functions in full. This
  design closes that asymmetry; it is not a new screen.
- **MCP full CRUD** (item A) and **Notify-target CRUD** (item C) are
  genuinely unbuilt — confirmed by reading `McpPanel`/
  `NotificationsPanel` in `main.rs`, both explicitly read-only today
  ("Add one with a `[[mcp_server]]` block... then restart the
  daemon").
- **Settings coverage expansion** (item E) — the tracking doc's own
  "operator-relevant knobs" framing can't be resolved from code (it's
  a product judgment about which of `aivyx.toml`'s ~35 config
  sections matter enough to expose); scoped with the user directly.

**One design problem genuinely unifies items A and C** (and, it turns
out, part of item E too): `aivyx-config/src/config_write.rs` (Chapter
U) already proves an in-place `toml_edit::DocumentMut` rewrite recipe
for *single* sections (`write_access_section`, `write_autonomy_
section`, `write_budget_section`, `write_profile_section`, `write_
voice_section` — all `load_document → patch in place → write_toml_
0600`), but nothing yet for **array-of-tables** (`[[mcp_server]]`,
`[[notify_target]]`, and — found during research —
`[[reflection_schedule]]`, which item E also needs). That primitive,
plus a secret-field handling convention, is "worth solving once,
deliberately."

## Scope

**In** — the shared array-of-table config-write primitive + secret-
field convention; MCP CRUD (incl. a test-connection probe); Notify-
target CRUD (`[[notify_target]]` + the shared `[email]` block) **and**
the channel adapters' own inbound bot tokens (`[telegram]`/
`[discord]`/`[slack]`, per explicit user decision — broader than this
design's own initial recommendation of outbound-only); the `schedule.
update`/`schedule.delete` autonomy-gating fix, including a stronger
protection than V09_PLAN row 8 literally asked for (see item D
below); Settings coverage expansion for `[embedding]`, `[memory]
profile`, `[proactive]`, and `[[reflection_schedule]]`'s primary
fields.

**Out** — MCP tool-level health (sub-project 8); every other
`aivyx.toml` section not named above (persona consolidation,
correction judgment, tool relevance, skill auto-propose, and ~25
others stay TOML-only — no operator ask names them); a separate
encrypted-secrets file (rejected below); sandbox/`bundled` fields on
`[[mcp_server]]` (advanced/internal, stay TOML-only).

One design doc. Given the size (4 substantial pieces sharing one
primitive), this is likely 2+ implementation plans rather than one —
that split happens at planning time, not here.

## A. The shared architecture

**Approaches considered:**

1. **(Chosen) Array-of-table helpers in `config_write.rs` + typed
   per-resource IPC messages.** Extend the existing module with
   `upsert_mcp_server_section`/`remove_mcp_server_section`,
   `upsert_notify_target_section`/`remove_...`,
   `upsert_reflection_schedule_section`/`remove_...` — each finds an
   existing `toml_edit::Table` in the array by its key field (`name`),
   replaces it in place or appends a new one. Pair with **typed
   `FrontendMessage` variants per resource** (`SetMcpServer`,
   `DeleteMcpServer`, `SetNotifyTarget`, `DeleteNotifyTarget`,
   `SetChannelAdapterToken`, `SetEmailConfig`, `SetEmbeddingConfig`,
   `SetProactiveConfig`, `SetReflectionSchedule`, `DeleteReflection
   Schedule`) — matching the existing `CreateSchedule`/`UpdateSchedule`/
   `DeleteSchedule` precedent, not a single generic writer.
2. **Rejected — one generic `PatchTomlSection{section_path, patch}`
   endpoint.** Smaller to build once, but needs its own bespoke
   allow-list to avoid becoming "write arbitrary config" (re-deriving
   approach 1's enumeration, just less legibly), and loses each
   section's own field-level validation (e.g. the budget range checks
   `write_budget_section` already does) that per-resource messages get
   for free by construction.
3. **Rejected — a separate `aivyx.credentials.toml`.** Real
   defense-in-depth idea, no existing precedent (every current secret
   — telegram token, email password, anthropic key — already lives
   inline in `aivyx.toml`), doubles the load/write surface. Per
   explicit user decision, out of scope for this pass.

**Secret-field convention** (binding on every piece below):

- A `GetX` query response never sends a secret's real value. It sends
  `{ configured: bool, source: "toml" | "env" | ... }` — mirroring
  `SourcedSecret`'s own `Debug` impl, which already redacts to
  `<redacted>`, and its `FieldSource` provenance field. The daemon-side
  wire type for this is a small new `RedactedSecret { configured: bool,
  source: String }` struct in `aivyx-ipc`, reused across every secret
  field (telegram/discord/slack tokens, email password, embedding
  api_key, MCP header/env values the operator marks sensitive).
- A `SetX` write message's secret field is `Option<String>`: `Some(new
  value)` writes it; `None` means "leave the existing TOML value
  untouched" (the daemon reads the current document, does not clear the
  key). This is standard password-change UX — the browser is never
  required to hold or resubmit a value it was never given.
- Every write in this design goes through the existing `.restart-
  banner` UX already shown by every other `config_write.rs` consumer
  (Settings/Teams/Roster) — MCP servers, channel adapters, and notify
  targets are all constructed once at daemon boot (confirmed:
  `crates/aivyx-cli/src/bin/aivyx.rs`'s startup loop over `mcp_
  servers`), so nothing here can take effect without a restart.

## B. MCP full CRUD

Add/edit/remove `[[mcp_server]]` entries from `McpPanel`
(`crates/aivyx-web/src/main.rs:3701`, currently read-only).

- **Fields exposed:** `name`, `transport` (stdio/sse/http — a picker
  that reveals the right fields), `enabled`. Stdio: `command`, `args`
  (list), `env` (key/value pairs, `${VAR}` placeholders written
  verbatim — resolution against the daemon's own environment already
  happens at load time, Studio doesn't need to resolve anything).
  SSE/HTTP: `url`, `headers` (key/value pairs, same `${VAR}`
  convention). `bundled` and `sandbox` are **not** exposed — internal/
  advanced, stay TOML-only (`bundled` is for Aivyx's own shipped
  servers; `sandbox` needs its own design per `docs/TOOL_SDK.md` §9,
  out of scope here).
- **Edit** resubmits the full entry (upsert-by-`name`) — no
  partial-field patching.
- **Test-connection probe:** reuses the *exact* connection logic the
  daemon uses at boot, not a separate simplified health check —
  `aivyx_mcp::McpServerBridge::start_with_sandbox` (stdio) or
  `SseTransport::connect`/`StreamableHttpTransport::connect` +
  `McpServerBridge::from_transport` (sse/http), called against the
  in-progress form's values *before* the operator saves. Reports
  success + tool count discovered, or the real connection error, then
  tears the probe connection down — it never joins the live server
  list (that only happens via a real save + restart).

## C. Notify-target + channel-adapter CRUD

Both the outbound routing config and the inbound bot tokens, per
explicit user decision (broader than this design's own initial
recommendation).

- **`[[notify_target]]` CRUD** (`NotificationsPanel`,
  `crates/aivyx-web/src/main.rs:2069`, currently read-only): `name`,
  `kind` (telegram → `chat_id`; webhook → `url`; email → `to`; web-ui →
  no fields), `enabled`, `is_default` (the loader already rejects >1
  default at load time — a `SetNotifyTarget` write surfaces that same
  `ConfigWriteError` rather than Studio pre-validating it client-side),
  `retry_count`/`retry_backoff_ms_start`/`rate_limit_max`/`rate_limit_
  window_secs` (advanced fields, shown with their existing defaults,
  editable).
- **Shared `[email]` SMTP block** — a singleton section (not
  array-of-table), so it reuses the simpler existing pattern
  (`write_voice_section`'s shape, not the new array-of-table one):
  `host`, `port`, `tls_mode`, `username`, `password` (secret,
  redacted-convention), `from`.
- **Channel adapters** (`[telegram]`/`[discord]`/`[slack]`): each
  gets its own dedicated `write_X_channel_section` function (the three
  structs aren't similar enough to force one generic writer). Exposes
  each platform's `token` (secret) plus its platform-specific fields —
  Telegram's `chat_filter`/`team_run_channel`/`team_trigger_rate_
  limit`/`team_command_allowed_senders`; Discord's and Slack's own
  fields, confirmed at plan time by reading their full struct
  definitions (not fully enumerated here — `TelegramConfig`'s shape at
  `crates/aivyx-config/src/lib.rs:1578` is the confirmed template).

## D. Schedule autonomy-gating fix

Small and separate from the screen work above — `SchedulesPanel`
already exists.

- Add the same `GrowthAdoption::None` check `schedule.create` already
  has to `ScheduleUpdateTool`/`ScheduleDeleteTool`'s `execute()` in
  `crates/aivyx-channel/src/schedule_tool.rs` — an agent's `schedule.
  update`/`schedule.delete` call fails with the same "the autonomy
  level does not permit self-scheduling" shape of error `schedule.
  create` already returns.
- **Additional protection, per explicit user decision** (stronger than
  V09_PLAN row 8's literal ask, and independent of the growth-tier
  check above): an agent's `schedule.update`/`schedule.delete` may
  **only ever target a schedule where `created_by == Agent`** — full
  stop, regardless of autonomy tier. Even at `BroadAuto`, an agent
  cannot touch an operator-created schedule. This mirrors the same
  instinct behind the existing agent-schedule count cap and frequency
  floor: the agent manages its own self-scheduling, never the
  operator's.
- **Not gated at all:** Studio's own `UpdateSchedule`/`DeleteSchedule`
  IPC path (confirm at plan time this is a distinct daemon-side
  handler from the `Tool` trait's `execute()`, not a shared code path
  — if it turns out to share code with the tool, the plan needs an
  explicit "is this call operator- or agent-initiated" signal instead
  of assuming the split is free). The operator must always be able to
  edit/delete any schedule via Studio regardless of autonomy tier.

## E. Settings coverage expansion

Primary fields only — advanced tuning knobs on each of these structs
stay TOML-only; confirmed at plan time by reading each struct's full
field list and marking non-primary fields explicitly excluded (not
guessed here). Extends `SettingsPanel`
(`crates/aivyx-web/src/main.rs`) and `SettingsSnapshot`
(`crates/aivyx-ipc/src/protocol.rs:1291` — confirmed today's snapshot
has no `[memory] profile` field at all, only a derived `embeddings_
available: bool`, so this is a genuine gap, not a UI-only miss).

- **`[memory] profile`** — `lite`/`smart` picker.
- **`[embedding]`** — `base_url`, `model`, `api_key` (secret
  convention). `dimensions`/`rag_top_k`/`rag_min_similarity`/`recall_
  window_turns`/`recall_gate_min_chars` stay TOML-only.
- **`[proactive]`** — `enabled`, `target` (a picker populated from the
  `[[notify_target]]` list this same design adds CRUD for — a direct
  synergy), `max_per_window`, `window_secs`. `signals` (which
  structural signal classes may surface) stays TOML-only.
- **`[[reflection_schedule]]`** — CRUD via the same array-of-table
  primitive as B/C: `name`, `cron`, `lookback_window_secs`, `enabled`.
  `role_override`/`skip_when_idle`/`min_audit_entries_to_fire` stay
  TOML-only.

## Testing

- **A (architecture):** unit tests on each new `config_write.rs`
  function mirroring the existing `access_writes_level_confirm_and_
  drops_stale_root`-style tests (temp TOML file, write, re-parse,
  assert the section round-trips and unrelated sections/comments
  survive byte-identical) — both the upsert-new-entry and
  replace-existing-entry paths for every array-of-table function.
- **Secret convention:** a test asserting a `GetX` response never
  contains a real secret substring for a config seeded with one, and a
  `SetX` write with `None` for a secret field leaves the existing TOML
  value unchanged (read the file after write, assert the old secret
  string is still present verbatim).
- **B (MCP CRUD):** round-trip tests per transport kind; the
  test-connection probe is DOM/network-interop-light enough that a
  plan-time decision is needed on how much of it is unit-testable
  (likely: the request/response shape is testable, an actual live MCP
  handshake is not, matching how this codebase already treats other
  live-connection code).
- **C (Notify-target/channel CRUD):** round-trip tests per notify kind
  and per channel adapter; a specific test for `is_default`'s
  at-most-one-default rejection surfacing correctly through the wire
  error.
- **D (schedule gating):** unit tests on `ScheduleUpdateTool`/
  `ScheduleDeleteTool::execute()` — `GrowthAdoption::None` rejects an
  agent call; an agent call targeting a `created_by == Operator`
  schedule rejects regardless of growth tier; an operator-initiated
  update/delete (via whatever the plan confirms is the real call path)
  is unaffected by either check.
- **E (Settings coverage):** round-trip tests per new section; a
  `[proactive] target` write referencing a nonexistent notify-target
  name surfaces a real validation error (matching the loader's own
  existing cross-reference validation for `[[trigger]].notify_targets`,
  confirmed against `crates/aivyx-config/src/lib.rs`'s existing
  cross-reference checks at plan time).
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test
  --workspace --exclude aivyx-desktop`, zero warnings/failures, plus
  `cargo build -p aivyx-web --target wasm32-unknown-unknown` and a
  rebuilt + committed `dist/` bundle per this repo's established
  convention.

## Out of scope

- MCP tool-level health signal (sub-project 8) — a different
  mechanism (audit-chain call-stat aggregation), only naturally
  co-located with the MCP CRUD screen, not part of this design.
- Any `aivyx.toml` section not explicitly named in items A-E above —
  confirmed there is no operator ask naming any of the other ~25
  sections; a future one becomes its own small follow-up, not guessed
  at here.
- MCP `sandbox`/`bundled` fields, and a general sandbox-preset picker
  UI (`docs/TOOL_SDK.md` §9's own future design).
- A separate encrypted-secrets file/store for these credentials — per
  explicit user decision, plaintext `aivyx.toml` + `0600` is the
  accepted trust boundary, matching every existing secret in this
  file today.
- Hot-reload/live-apply of any section this design touches — every
  write here requires a daemon restart, matching every existing
  `config_write.rs` consumer; no new live-reload mechanism is built.

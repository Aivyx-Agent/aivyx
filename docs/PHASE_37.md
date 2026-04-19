# Phase 37 — WebFetchTool Hardening

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close all three web-fetch deferrals in one phase:

1. **Binary response bodies** — non-UTF-8 responses return
   base64-encoded body instead of failing.
2. **Non-GET verbs** — new `WebPostTool` for POST/PUT/PATCH/
   DELETE with `net.post` scope and request body support.
3. **Redirect following** — opt-in `follow_redirects` field
   with per-hop scope re-check via manual redirect loop.

Cuts the rolling deferral backlog from 6 to 3.

## Why now

1. **Three oldest deferrals.** All three date to Phase 12 Q1/Q5
   (2026-04-16). They've survived 25 phases and 6 cleanup
   rounds untouched because each was correctly deferred — the
   agent had more pressing structural work. That structural
   work is done.

2. **Practical agent capability.** A research agent that can't
   POST to an API, can't handle binary downloads, and can't
   follow redirects is meaningfully limited. These are the
   three most common "why can't I..." moments.

3. **Self-contained scope.** All three changes live in
   `web_fetch.rs` (or a new sibling file). No architecture
   changes, no new crates, no contract amendments.

## Design decisions

- **Separate `WebPostTool`, not a widened `WebFetchTool`.**
  Different scope bases (`net.fetch` vs `net.post`), different
  trust tiers (SemiTrusted-accessible vs Trusted-only),
  different input shape (POST needs a `body` field). Separate
  tools make the security boundary structural. The LLM sees
  different tool names/descriptions, reducing misuse.

- **Binary bodies: base64 fallback, not a new StreamEvent
  variant.** When the response body is not valid UTF-8, return
  `"body_encoding": "base64"` and base64-encode the body in
  the `body` field. Skip `StreamEvent::ToolOutput` streaming
  for binary responses (since it's `&str`-only). UTF-8
  responses continue to stream and return `"body_encoding":
  "utf-8"` (or omit the field for backward compatibility).
  This avoids touching `lib.rs`.

- **Redirect following: manual loop, not reqwest policy.**
  `reqwest::redirect::Policy` callbacks are sync-only and
  can't perform async scope re-checks. Instead, `execute()`
  implements a manual redirect loop: detect 3xx, extract
  `Location`, re-derive `required_scope` from the new URL,
  re-check against the agent's held capabilities, and issue
  a new request. Maximum 10 hops. Default off (`follow_
  redirects: false`) to preserve existing behavior.

- **Scope re-check needs capabilities in execute().** The
  manual redirect loop needs to check whether the redirect
  target is within the agent's granted capabilities. The
  agent's effective `CapabilitySet` is not currently available
  in `ToolContext`. Rather than adding it to `ToolContext`
  (which would touch `lib.rs`), the tool stores the effective
  capabilities at construction time via an `OnceLock` pattern
  — the binary sets it after capability assembly, same pattern
  as `MissionCreateTool::set_mission_store`.

- **`WebPostTool` shares the reqwest client with
  `WebFetchTool`.** Both tools are built from the same
  `WebFetchToolConfig` builder. The POST tool accepts
  `method` (POST/PUT/PATCH/DELETE), `url`, `body` (string or
  JSON), `content_type`, and `timeout_ms`.

## Streak predictions

- **DESIGN.md** -- Very low risk. No architecture change.
  Prediction: **untouched** (streak at 8 from Phase 37).

- **PRODUCT.md** -- Low risk. No product commitment change.
  Prediction: **untouched** (streak at 2).

- **Production-core `aivyx-core/src/lib.rs`** -- At risk from
  the redirect scope re-check if we chose option (A). With the
  OnceLock pattern, `lib.rs` stays untouched.
  Prediction: **untouched** (streak at 6 from Phase 37).
  Reality: **touched** in Task 3 — `pub use` re-export line
  extended with `WebPostTool, WebPostToolConfig`. Streak
  resets to 0.

## Tasks

### Task 1 -- Open commit + PHASE_37.md scaffold

This file. Update `docs/README.md` to show Phase 37 as Open.
Update `docs/ROADMAP.md` with Phase 37 active pointer.

### Task 2 -- Binary body support (base64 fallback)

Modify `WebFetchTool::execute` in `web_fetch.rs`:

- When `String::from_utf8` fails, base64-encode the raw bytes
  instead of returning `ToolOutcome::Failed`.
- Add `body_encoding` field to the output JSON: `"utf-8"` for
  text responses, `"base64"` for binary.
- Skip `StreamEvent::ToolOutput` streaming for binary
  responses (the chunk type is `&str`).
- Update the tool description to mention binary support.
- Tests: binary response returns base64, UTF-8 response
  unchanged.

### Task 3 -- WebPostTool

New tool in `web_fetch.rs` (or `web_post.rs` if the file
gets too large):

- Input schema: `url` (required), `method` (optional, default
  "POST", allowed: POST/PUT/PATCH/DELETE), `body` (optional,
  string or JSON), `content_type` (optional, default
  "application/json"), `timeout_ms` (optional).
- Scope: `net.post:<url>` — uses the existing `net.post`
  base from `KNOWN_BASES`.
- Same defense-in-depth pattern as `WebFetchTool`:
  `required_scope` derives from the URL, `execute` re-parses.
- Same body cap, same streaming, same binary fallback.
- Register in binary alongside `WebFetchTool`.
- Backcompat floor: add `net.post` for Local channel.

### Task 4 -- Redirect following with per-hop scope re-check

Add `follow_redirects` boolean field to `WebFetchTool`'s
input schema (default `false`).

When `true`, `execute()` implements a manual redirect loop:

1. Issue the request.
2. If 3xx with `Location` header, extract the redirect URL.
3. Re-derive scope from the redirect URL.
4. Re-check scope against the agent's effective capabilities
   (stored via `OnceLock` on the tool).
5. If scope check passes, issue a new GET to the redirect URL.
6. Repeat up to 10 hops.
7. If scope check fails, return the redirect URL in the output
   with a clear "redirect denied by scope" message.

Also add `follow_redirects` to `WebPostTool` with the same
behavior.

Binary wiring: set the effective capabilities on both tools
after `assemble_role_envelope` completes.

### Task 5 -- Tests

- Binary body: base64 encoding, content_type preserved.
- WebPostTool: POST/PUT/PATCH/DELETE verbs, body handling,
  scope derivation, missing URL errors.
- Redirect following: 301/302 followed within scope, redirect
  to out-of-scope URL denied, redirect chain capped at 10.
- Capability integration: `net.post` scope grants/denies.

### Task 6 -- Exit freeze + docs

## Exit criteria

- [x] Binary body support: base64 fallback when UTF-8 fails,
      `body_encoding` field in output JSON, streaming skipped
      for binary. 2 tests.
- [x] `WebPostTool`: POST/PUT/PATCH/DELETE via `net.post` scope,
      JSON + string body, Trusted-only ceiling. 9 tests.
- [x] Binary wiring: tool registered alongside `web.fetch`,
      `net.post` in backcompat floor, 2 registration tests.
- [x] Redirect following: `follow_redirects` boolean on both
      tools, manual loop with per-hop scope re-check via
      `OnceLock<CapabilitySet>`, max 10 hops, binary wiring
      via `set_effective_capabilities`. 6 tests.
- [x] All 788 tests pass (up from 771 at Phase 36 exit).
- [x] `DESIGN.md` untouched (streak at 13 from Phase 25).
- [x] `PRODUCT.md` untouched (streak at 2 from Phase 36).
- [x] `aivyx-core/src/lib.rs` touched in Task 3 (pub-use
      re-export for `WebPostTool`, `WebPostToolConfig`).
      Streak resets to 0.

## Prediction vs reality

| Streak target | Predicted | Reality | Notes |
|---|---|---|---|
| DESIGN.md | untouched (8→13) | untouched | continues |
| PRODUCT.md | untouched (2) | untouched | continues |
| lib.rs | untouched (6) | **touched** | `pub use` re-export for `WebPostTool` |

## Deferral status

Phase 37 clears 3 of the 6 rolling deferrals:
- ~~Binary response bodies~~ (Phase 12 Q5) → Task 2
- ~~Non-GET verbs~~ (Phase 12 Q1) → Task 3
- ~~Redirect following~~ (Phase 12 Q1) → Tasks 4-5

Remaining 3 deferrals:
1. **Rendering parity** — terminal vs Telegram output formatting
2. **Integration test infra** — E2E test harness for full turns
3. **Protocol versioning** — MCP/SSE stream format negotiation

## Ship records

- **Task 1** `582727b` — open commit, PHASE_37.md scaffold,
  README.md + ROADMAP.md updates.
- **Task 2** `5023eb8` — binary body support in `WebFetchTool`
  (base64 fallback, `body_encoding` field, 2 new tests). Added
  `base64 = "0.22"` workspace dep. 773 tests.
- **Task 3** `a1bc08b` — `WebPostTool` (POST/PUT/PATCH/DELETE),
  `net.post` scope base, binary registration, backcompat floor,
  2 registration tests, 7 tool tests. 782 tests.
- **Tasks 4+5** `aed97cc` — redirect following with per-hop
  scope re-check. `OnceLock<CapabilitySet>` on both tools,
  `follow_redirects` input field, shared helpers
  (`extract_redirect_location`, `check_redirect_scope`,
  `collect_body`). Binary wires capabilities via
  `set_effective_capabilities`. 6 redirect tests. 788 tests.

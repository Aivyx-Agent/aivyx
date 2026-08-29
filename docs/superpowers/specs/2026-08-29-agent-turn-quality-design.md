# Agent turn-quality fixes (POLISH_WAVES.md sub-project 4) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 4 bundles 8 findings from `VITRINE.md`
§2/§2b's chat stress-testing round, all sharing one theme: the turn loop
completes and returns *something*, but that something is degraded in a
way the operator has to notice by reading closely (a thrashing tool
retry, a bare JSON leak, a silently dropped fact, a subtly wrong
identifier). None of these are new screens — this is backend/prompt-logic
work, plus one Studio line-rendering fix and one docs-only item. Grounded
directly against the real, current code (not `VITRINE.md`'s own
2026-07-05 line numbers, which predate several unrelated refactors), per
this workspace's now-standard discipline.

## Scope

**In** — all 8 items below. **Out** — anything not already named as one
of the 8; in particular Nonagon templates, voice, and the UI Modernization
pass stay in their own sub-projects (2, 6) per `POLISH_WAVES.md`'s own
exclusions.

One design doc, one implementation plan, one SDD execution — matching the
`classic-retirement` chapter's precedent for a multi-piece sub-project
(there: 5 components A–E; here: 8, several of them one-function or
one-line).

## A. Tool-failure thrash nudge

**Finding:** with `web_search` down, the model pivoted reasonably once,
then degenerated into 6 failed searches and a raw `web.fetch` of the
search engine's own homepage — never reporting the outage. Bridle's
existing breaker (`crates/aivyx-core/src/agent.rs`, `note_repeat`) only
catches *identical* consecutive calls (same `tool_id` + same `input`
signature); these calls all differed, so it never fired.

**Where it lives:** `LlmPlanner::observe_tool_outcome`
(`crates/aivyx-core/src/llm_planner.rs:1517`), the exact seam where a
`ToolResult` message is already appended to LLM history after every
dispatched call. `render_tool_result` (line 1630) already computes an
`is_error: bool` per outcome — `Completed` is the only non-error variant;
`Denied`/`NotInRole`/`RateLimited`/`RequiresEscalation`/`Failed` are all
`true`. The method signature already takes `tool_id: ToolId` (currently
unused, prefixed `_tool_id`) — the plumbing to key off tool identity is
already present, just not read.

**Design:** add `consecutive_tool_failures: (Option<ToolId>, usize)` state
to `LlmPlanner`. On each `observe_tool_outcome` call: if `is_error` and
the `tool_id` matches the stored one, increment; if it doesn't match (a
different tool, or the first failure), reset to `(Some(tool_id), 1)`; any
non-error outcome resets to `(None, 0)`. When the count reaches **3** and
this is the call that just crossed the threshold (not every call after),
append one additional `LlmMessage` after the `ToolResult` — a `System`-
role advisory (matching whatever `LlmMessage` variant the codebase already
uses for injected guidance; confirm the exact variant during planning) with
text: `"The {tool_name} tool has failed 3 times in a row. Stop retrying it — report the outage to the operator instead of trying an unrelated approach."`
`tool_name` resolved the same way the turn loop already resolves it for
Candor's `called_tools` (`self.tools.get(o.tool_id).map(|t| t.name())`) —
`LlmPlanner` needs the same registry handle or the name passed in;
resolve during planning which is more natural given `LlmPlanner`'s actual
fields.

Fires **once** per streak (the threshold-crossing call only) so a
still-failing 4th, 5th, 6th call doesn't repeat the nudge. Does not halt
the turn — `MAX_STEPS_PER_TURN` and the existing breakers remain the
backstop if the model ignores the nudge.

## B. gpt-oss finishing-family gaps + a universal reply floor

**Finding (two related but independent problems):** (1) `gpt-oss:20b`
produced a bare JSON object of tool *arguments* as its final answer, and
separately produced empty completions — its `ollama_prompt_strategy`
resolves to `None` because `detect_model_family` doesn't recognize the
`gpt-oss` prefix at all (`crates/aivyx-config/src/lib.rs:3581`). (2) The
Studio and other frontends have no fallback when a turn's `final_message`
is genuinely empty or is a leaked tool-args object — the operator sees
nothing.

**B.1 — family detection.** Add a `gpt-oss` branch to
`detect_model_family` (`crates/aivyx-config/src/lib.rs`), following the
existing `qwen`/`gemma`/`llama` pattern: `model.split(':').next()` already
isolates `"gpt-oss"` from `"gpt-oss:20b"`; no digit-extraction needed
(unlike qwen/gemma/llama, `gpt-oss` has no numbered generations to date —
match the literal string `"gpt-oss"`). Add
`"gpt-oss" => OllamaFamilyStrategy::FewShotExamples` to
`default_for_family` (line ~3536) — reusing the existing lever already
proven for qwen3/gemma4 rather than inventing a new `OllamaFamilyStrategy`
variant. This is a re-targeting of an existing mechanism, not new
substrate: qwen3/gemma4's few-shot examples counter *refusal-to-call*;
gpt-oss's problem is *post-tool finishing*, but "show worked examples of
correct behavior" is the same lever pointed at a different failure mode.
No new config surface; operators can still override via
`[ollama.prompt_strategies] "gpt-oss" = "..."` like any other family.

**B.2 — universal reply floor.** A **family-independent** safety net in
the turn loop, `crates/aivyx-core/src/agent.rs`'s `LoopOutcome::Completed`
arm (~line 660, right where Candor's `detect_unfulfilled_claims` already
runs against `final_message`). Before Candor's check, apply:

```rust
fn floor_unusable_final_message(msg: &str) -> Option<&'static str> {
    let trimmed = msg.trim();
    if trimmed.is_empty() {
        return Some("I wasn't able to produce a usable reply this turn — please try again.");
    }
    // A bare tool-args leak: the entire message parses as a JSON object
    // (not an array, not a scalar — tool arguments are always objects).
    if let Ok(serde_json::Value::Object(_)) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some("I wasn't able to produce a usable reply this turn — please try again.");
    }
    None
}
```

Applied to *all* models, not just detected gpt-oss — protects any
current or future family that hits the same failure shape without
needing its own detection rule. Deliberately conservative: only fires on
empty-after-trim or a message that is *entirely* a JSON object (a real
prose reply that happens to mention JSON inline won't parse as one via
`serde_json::from_str` on the whole trimmed string, so it won't
false-positive).

## C. Studio "(no reply)" rendering

**Finding:** the Chat surface renders nothing when a turn completes with
empty streamed text — indistinguishable from "still working" or a bug.

**Where:** `crates/aivyx-web/src/main.rs`, the `DaemonEnvelope::TurnComplete`
arm (~line 8104):

```rust
DaemonEnvelope::TurnComplete { .. } => {
    let text = streaming();
    if !text.is_empty() {
        transcript.write().push(ChatLine::assistant(text));
    }
    streaming.set(String::new());
}
```

Add an `else` branch pushing `ChatLine::system("(no reply)".to_string())`.
Stays useful independently of B.2: B.2 floors the turn loop's own
`final_message`, but Studio's `streaming` signal accumulates from
`StreamEventPayload::Text` events specifically — a turn that emitted only
`Status`/`ToolCall` events and then completed (no `Text` event at all)
would still reach this branch with an empty `streaming()` even after B.2
ships, because B.2's floor text is only guaranteed to reach the *daemon's*
`final_message`, and the wiring from `final_message` to a `Text` stream
event is a separate code path to confirm during planning (not assumed
here).

## D. Chat tool-call argument rendering parity

**Finding:** Studio chat lines show only the tool name; mission/cron
turns already journal full arguments.

**Where:** `crates/aivyx-web/src/main.rs`, the
`StreamEventPayload::ToolCallStarted` arm (~line 8100), currently:

```rust
StreamEventPayload::ToolCallStarted { tool_name, .. } => {
    transcript.write().push(ChatLine::system(format!("→ {tool_name}")));
}
```

The proven format already exists —
`StreamEventPayload::render_for_cli()` (`crates/aivyx-ipc/src/protocol.rs`)
does `format!("  → {tool_name} {input_oneline}\n")` where `input_oneline`
is `serde_json::to_string(input)`. `ToolCallStarted` already carries
`input: serde_json::Value` (`protocol.rs:2426`) — it's discarded today via
`..`. Change to:

```rust
StreamEventPayload::ToolCallStarted { tool_name, input, .. } => {
    let input_oneline = serde_json::to_string(&input).unwrap_or_default();
    transcript.write().push(ChatLine::system(format!("→ {tool_name} {input_oneline}")));
}
```

(No leading two-space indent — that's `render_for_cli`'s CLI-specific
formatting; chat's own `ChatLine::system` rendering supplies its own
layout.)

## E. Identifier-fidelity check (Candor family)

**Finding:** three independent repros of a stochastic one-character
identifier corruption (VH-EZT→VH-EQT, a METAR wind group misquoted, an
ICAO-transposition family) — the source was verifiably correct in memory
or tool output; the model's reply subtly altered it.

**Design:** a new function alongside (not inside) Candor's
`crates/aivyx-core/src/claim_check.rs` — different shape than its
phrase-matching `RULES` table, so a new function in the same file (or a
sibling module if the file's own conventions favor that; decide during
planning by looking at the file's current size/organization). Wired at
the same site as B.2 and Candor's existing check
(`LoopOutcome::Completed` arm, `agent.rs`):

1. **Build the identifier pool** from this turn only (not global memory):
   tokenize the `output` field of every `ToolOutcome::Completed` this turn
   (available via `observed`, same as Candor's `called_tools` derivation)
   for identifier-shaped substrings — pattern: alphanumeric-and-hyphen
   runs, length ≥ 4, containing both at least one ASCII letter and one
   ASCII digit (this excludes plain English words and very short tokens
   like "the" or "V1", while catching tail numbers, METAR groups, ICAO/
   part-number-style codes).
2. **Tokenize `final_message`** the same way.
3. For each final-message token **not** exactly present in the pool,
   check Levenshtein/edit distance against every pool token (no existing
   crate-wide helper — `aivyx-toolkit/src/budget_store.rs` has one for an
   unrelated purpose in a crate `aivyx-core` doesn't depend on; write a
   small local one, or a cheap distance-exactly-1 check via single-edit
   enumeration rather than full DP, since only "is it 1" matters here,
   not the exact distance).
4. On a distance-exactly-1 match, append an honest note (Candor's own
   pattern: `"⚠ {note}"`), e.g.:
   `"⚠ I wrote '{token}' but the source said '{closest}' — please double-check this identifier."`

Conservative by construction: only compares against identifiers the turn
itself surfaced (bounded pool, bounded false-positive surface), only
flags an exact single-edit mismatch (not "similar," which would be noisy),
and never blocks the turn — same non-blocking posture as Candor's
existing checks.

## F. Source-currency instinct

**Finding:** a GA-airports answer mixed 1930s-defunct fields from an
undated Wikipedia list with current ones, and omitted the one real
current airport.

**Design:** one added sentence in `DEFAULT_SYSTEM_PROMPT` (Chapter Keel,
`crates/aivyx-config/src/lib.rs:216`), under the existing "How you work"
bullet list — **not** a new prompt subsystem. The charter is deliberately
capped (`default_charter_carries_its_invariant_pillars` test asserts
`DEFAULT_SYSTEM_PROMPT.len() < 2000` bytes specifically to protect small
local models' context budgets); current length is well under that
ceiling, so one sentence fits without approaching it, but the addition
should stay to a single sentence to honor the same discipline. Proposed
line, appended to the existing "Prefer acting with your tools…" bullet or
as its own bullet (decide phrasing/placement during planning by reading
the current bullet list fresh):

> "Treat undated source listings as unverified for currency — flag
> entries that may be outdated rather than presenting them as current."

No new test assertions required beyond the existing length ceiling test
continuing to pass; optionally add one keyword-presence assertion
(`has("current")` or similar) to
`default_charter_carries_its_invariant_pillars`, mirroring how the test
already pins the other invariant pillars.

## G. Volunteered-fact persist gap

**Finding:** Chapter Thread's history replay now lets a volunteered
answer to the agent's own question *connect* conversationally, but the
fact is still never `memory.write`-persisted — only Etch's explicit
"remember this" phrasing triggers a deterministic save.

**Where:** `crates/aivyx-channel/src/memory_recall.rs`'s
`SemanticMemoryContext` already holds a `conversation_windows` handle
(Chapter Thread's own replay mechanism, `conversation_window.rs`) and
already runs `capture_explicit_memory(user_message)` unconditionally at
the top of `ContextProvider::recall` (line ~532), storing under the
existing `EXPLICIT_MEMORY_TOPIC` ("operator-notes") via a store→embed
path.

**Design:** add a second trigger condition, checked in the same
`recall()` hook, before or alongside `capture_explicit_memory`:

1. Look up the session's `ConversationWindow` (via the existing
   `conversation_windows` handle) and read its **last** entry.
2. If that entry's `Role` is the assistant's and its text (trimmed) ends
   in `?`, treat the *current* `user_message` as a candidate answer.
3. Persist `"Q: {question} A: {answer}"` under `EXPLICIT_MEMORY_TOPIC`
   via the same store→embed call `capture_explicit_memory` already makes
   (factor the store→embed body into a small shared helper both call, so
   there's one persistence path, not two).

Conservative choices carried over from Etch's existing design: best-
effort (logs + swallows errors, never panics — it's in the per-turn hook
path), and skip when the candidate answer is itself very short/low-signal
(reuse whatever minimum-length heuristic, if any, `capture_explicit_memory`
or the recall gate already applies, rather than inventing a new one) —
confirm the exact reuse during planning by reading the current gate
logic fresh.

## H. Keyed-backend guidance docs

**Finding:** the bundled DuckDuckGo backend silently 202-blocks on this
rig (fixed to fail loudly by an earlier polish item, per
`POLISH_WAVES.md` sub-project 1) — no operator-facing guidance exists for
switching to a keyed backend.

**Design:** docs-only. Add a short section to `docs/TOOLS.md` (the
existing tool-catalog doc, natural home for backend-configuration
guidance) covering: DuckDuckGo's zero-config default and its known
bot-blocking behavior (HTTP 202) under sustained/automated use, and how
to configure Brave Search or SerpAPI as a keyed alternative (whatever
config keys the existing `web_search` tool already recognizes for those
backends — read `crates/aivyx-core`'s `web_search`/`web.rs` tool
implementation during planning to confirm the exact config field names
rather than guessing them here).

## Testing

- **A** (tool-failure nudge): unit test on `LlmPlanner::observe_tool_outcome`
  — 3 consecutive error outcomes for the same `tool_id` append exactly one
  extra advisory message after the 3rd `ToolResult`; a differing `tool_id`
  or an intervening success resets the streak (no nudge at count 3 after a
  reset); a 4th, 5th consecutive failure does not append a second nudge.
- **B.1**: extend `crates/aivyx-config/src/tests.rs`'s existing
  `detect_model_family`/`resolve_ollama_prompt_strategy` test group with
  `gpt-oss:20b` → family `"gpt-oss"` → `FewShotExamples`, mirroring the
  existing qwen3/gemma4/llama3 cases.
- **B.2**: unit tests on `floor_unusable_final_message` (or equivalent) —
  empty string, whitespace-only, a bare `{"path": "...", ...}` object all
  floor; ordinary prose (including prose that contains inline `{...}`
  text as a quoted substring, not as the *entire* trimmed message) does
  not.
- **C, D**: `cargo check -p aivyx-web` / `cargo clippy -p aivyx-web
  --all-targets -- -D warnings` (both confirmed to work natively, no
  wasm32 target needed, from the classic-retirement chapter) plus a
  rebuilt+committed `dist/` bundle via the rustup wasm32 toolchain
  (also discovered that chapter) if these two land as real UI-visible
  diffs — confirm during planning whether either needs a dedicated
  behavioral test (e.g. a `ChatLine` construction unit test) or whether
  compile+clippy is the appropriate bar, matching how prior Studio
  line-rendering changes in this workspace were tested.
- **E**: unit tests on the new identifier-fidelity function — a pool
  containing "VH-EZT" and a final message containing "VH-EQT" flags;
  identical tokens don't flag; tokens absent from the pool with no
  near-miss don't flag; a short common word never enters the candidate
  pool (length/digit-presence filter holds).
- **F**: `default_charter_carries_its_invariant_pillars` continues to
  pass (length ceiling); optionally extended with a new keyword
  assertion for the added sentence.
- **G**: unit test on the new trigger — a `ConversationWindow` whose last
  entry is an assistant message ending in `?`, followed by a `recall()`
  call with a plausible answer, results in a persisted
  `"Q: ... A: ..."` entry; a last entry that doesn't end in `?`, or an
  empty window (fresh session), does not trigger the second path (only
  Etch's existing explicit-phrase path can fire).
- **H**: no tests — docs-only.
- Full sweep before merge, matching every prior chapter this session:
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`, zero warnings/failures.

## Out of scope

- Any new `OllamaFamilyStrategy` variant — B.1 reuses `FewShotExamples`.
- Any change to Bridle's existing consecutive-identical-call breaker —
  A adds a parallel, differently-keyed mechanism, not a modification to
  Bridle.
- Any UI beyond the two one-block changes in C and D — no new screens,
  matching `POLISH_WAVES.md`'s own framing ("one Studio line-rendering
  change only" — this design has two, both in the same category).
- Global memory-wide identifier verification (E is turn-scoped only, by
  design, to bound cost and false positives).
- Expanding the charter beyond F's one sentence, or restructuring it.

# Through the Shop Window — the operator walkthrough (Chapter Vitrine)

> **Status: IN PROGRESS (resumed 2026-07-05 on the rebuilt rig — clean
> v0.8.0 end-user install, operator logged into the Studio over the
> LAN).** The v0.9 polish backlog, captured
> live while the operator walks every surface of the real agent's Studio
> (rig, SSH tunnel, real data). Each finding gets a severity —
> **P1** broken/lying, **P2** friction/confusing, **P3** cosmetic —
> and feeds the polish waves (V09_PLAN #4).

## Findings

### 0 · Pre-walkthrough (the Gatehouse baptism attempt)
- **P2 — host firewalls are invisible in the exposure story.** UFW on the
  rig allowed 22 and silently dropped 7843: the operator saw a browser
  that "never populated" with zero feedback, while every Aivyx-side
  check was green (bind, token, cookie, /ws upgrade). Fix candidates:
  INSTALL.md + GATEHOUSE.md mention distro firewalls next to the bind
  knobs; possibly a startup hint when binding off-loopback ("if clients
  can't connect, check the host firewall").
- **Positive: Postern + Gatehouse verified live from the LAN** — 401
  without/with-wrong token on both the page and `/ws`, 200 with the
  token, cookie planted correctly, interlock permitted the bind only
  because the token existed.
- **P2 (2026-07-05 rebuild baptism) — the native Basic-auth prompt is a
  silent dead end when credentials go stale.** The operator's browser
  had saved a wrong token; every "Sign In" silently re-submitted the
  saved value regardless of what was pasted, and the only feedback was
  the prompt re-appearing — no "wrong token" indication anywhere, and
  nothing in the daemon journal either (had to packet-capture to
  diagnose). Fix candidates: log a rate-limited `web ui: rejected token
  from <ip>` line server-side; longer-term consider a tiny login *page*
  (styled, explicit error text, sets the cookie) instead of the bare
  browser prompt — password managers treat those far more predictably.
- **Positive: the rebuilt on-ramp put the operator into the Command
  Center from a cold, clean install in one session** (public installer →
  wizard → Gatehouse token → browser login).
- **P1 (2026-07-05) — LAN Studio loads but every screen is empty unless
  `web_ui_allowed_origins` is set.** The `/ws` CSWSH origin gate only
  allows loopback origins by default, so a browser at
  `http://<lan-ip>:7843` passes page auth but gets 403 on the data
  socket — the UI renders beautifully and shows nothing, with no error
  surfaced anywhere (client console only). The operator config that
  *enables* LAN exposure (`web_ui_host` + token) doesn't imply the
  origin, which it always could: the served host:port is a safe origin
  to auto-allow (a DNS-rebinding page still presents its own hostname).
  Fix candidates: (a) auto-allow the request's own `Host` header origin
  when it matches the bound port, (b) config-load hint when
  `web_ui_host` is non-loopback and `allowed_origins` is empty, (c) the
  Studio should *display* a "connection rejected" state instead of
  silent empty screens. Workaround applied on the rig:
  `web_ui_allowed_origins = ["http://10.80.80.148:7843"]`.

### 1 · Command Center
- **Positive: information and data all correct and accurate** on a
  day-old agent (operator verdict, 2026-07-05).
- **P3 — dashboard cards look "flat and boxy, similar to every other
  agentic dashboard".** Operator wants a distinctly more modern layout/
  look/style, not just token tweaks. Direction: revisit the Stitch
  brand screen images (~/BACKUP/AI-Project/aivyx-brand) for unrealized
  design intent, plus fresh web research on modern dashboard treatments
  (depth/elevation, gradients/glass, asymmetric layout, motion). This is
  the headline item for the Command Center polish wave.

### 2 · Chat
- **Positive: honest non-answer on missing memory** — "what's my home
  airport?" → 3× memory.search → "I'm not sure, could you let me know?"
  No confabulated airport. Evidence-discipline working.
- **P2 (agent behavior) — a volunteered fact answering the agent's OWN
  question is not captured.** Operator: "Where I do my training from is
  Jandakot" → reply was a canned "let me know what you'd like to tackle
  next": no acknowledgment, no memory.write, fact lost. Etch's
  deterministic hook only catches explicit "remember this" phrasing;
  the ask→answered→persist loop needs closing (candidate: when the
  agent just asked a question, treat the operator's next message as a
  candidate answer and bias the memory hook accordingly). Root cause
  amplified by the fresh-context finding below — the agent literally
  cannot see that it just asked a question. **HALF-FIXED by Chapter
  Thread (2026-07-05): with history replay the answer now CONNECTS**
  ("Your home airport is Jandakot Airport") — but the fact is still
  not memory.written, so the persist half stays open. Same probe also
  reproduced the never-invent pattern: the reply appended an
  unprompted, confabulated "(YJAT)" code.
- ~~P3 — 3 identical memory.search calls in one turn~~ **CORRECTED by
  the audit chain (2026-07-05): the three searches were DISTINCT
  queries** ("airport", "home airport", "base") — reasonable refinement,
  not a loop. The real finding: **P3 — Studio chat tool lines render
  only the tool name, never the arguments**, so distinct calls look like
  stuck repetition to the operator. Mission/cron turns journal full args
  (`→ web_search {"query": …}`); chat should render the same.
- **P1 (2026-07-05 investigation) — same-session follow-ups fail by
  design on the Chat surface.** "Whats the ICAO for Jandakot?" →
  web_search → *(nothing rendered)* → "Did you find the correct code?"
  → the agent guessed "code" meant a code snippet/repo — zero knowledge
  of its own previous turn. Audit chain confirms both turns share one
  session; the cause is the WI.2 design: turns are fresh-context (no
  transcript replay), and the Phase 86 conversation window feeds only
  recall *relevance*, never the model prompt. Fine for headless
  automation; on a chat UI it breaks the most basic conversational
  expectation (pronouns, ellipsis, "did you find it?"). **DECIDED +
  BUILT same-day (Chapter Thread, docs/THREAD.md): full history
  replay, on by default** — the operator chose real user/assistant
  message replay over block injection; `[agent]
  conversation_history_turns` (default 8, 0 restores fresh-context).
  Trigger turns structurally unaffected. Rig verification pending.
- **P1 (2026-07-05 investigation) — the bundled web-search backend was
  silently dead all day.** DuckDuckGo answers bot-flagged traffic with
  HTTP **202** + a challenge page; the zero-config backend parsed that
  to `[]` with no error, so every `web_search` today (operator chat +
  the 07:16/07:30 trend-scans) returned "no results" indistinguishable
  from a real zero-hit. Downstream: the ICAO turn ended with an empty
  completion (the operator saw nothing), and a headless repro of the
  same question **confabulated "YJND"** (real answer: YPJT) tagged
  "source unknown" — a never-invent violation triggered by garbage-in.
  **FIXED same-day (backend honesty):** non-200 from DDG (and non-2xx
  from Brave/SerpAPI) now returns an explicit tool error saying the
  backend is unavailable and results are NOT empty-because-no-matches.
  Still open: the 202 means DDG is blocking this rig — keyed backend
  (Brave/SerpAPI) guidance for operators, and the Chat surface should
  render *something* (e.g. "(no reply)") when a turn completes with
  empty text instead of a silent void (routine turns also often end
  with `final_message: ""` on gpt-oss:20b — same phenomenon).

### 2b · Chat stress round (2026-07-05, post-Thread piped sessions)
- **Positives: the conversational fabric holds.** Three-turn pronoun
  chain ("which of those… and what is ITS code?") resolves against the
  agent's own prior answers; a correction turn ("sorry, I meant 2400")
  recomputes correctly; a fresh session knows nothing of other sessions
  (no replay leak) and says so honestly; "remember this" + "what did I
  just ask you to remember?" round-trips through Etch + replay.
- **P1 — the planner's pruning budget ignored the real context window.**
  Ollama's planner budget was the provider-class default (8k) even with
  an explicit `[ollama] num_ctx = 16384` — wrong in both directions
  (premature pruning on normal turns, and no protection on fat ones).
  FIXED: explicit num_ctx is now authoritative for the planner budget.
- **P1 — a giant tool result was un-prunable and silently truncated the
  system prompt.** One raw `web.fetch` of a full HTML page became a
  single ~30k-token ToolResult message; the pruner can only drop whole
  messages and must keep the tail, so the request sailed past the real
  16k window and Ollama truncated server-side — from the front, where
  the charter/security prompt lives. Audit showed a turn at 32,015
  estimated tokens. FIXED: tool-result content is capped at ~half the
  context window (floor 4k chars) with an explicit truncation marker.
- **P2 (agent behavior, open) — tool-failure thrash.** With web_search
  down (DDG re-blocked mid-test — the new honest error fired), the
  model adapted well at first (pivoted to web_read on Wikipedia) but
  then degenerated: 6 failed searches, raw web.fetch of duckduckgo.com
  itself, and a final answer that was an inventory of a page's <img>
  tags — off-task, no ICAO answer, no "search is unavailable" report.
  Candidate: after N consecutive failures of one tool, inject a nudge
  to stop and report the outage (Bridle's breaker only catches
  identical repeats; these calls all differed).
- **P2 (agent behavior, open) — tool-call-shaped JSON escapes as the
  final answer.** Post-fix re-test: a turn ended with the final message
  being a bare JSON object of tool ARGUMENTS
  (`{"path":"…airports.csv","delimiter":",",…}`) — the model emitted a
  call it never dispatched and the turn treated it as the reply; chat
  would show naked JSON. Same family as the empty-completion issue
  (gpt-oss:20b post-tool finishing; its `ollama_prompt_strategy` is
  "none — family: undetected", so it gets no finishing scaffold the way
  qwen3's few_shot strategy provides). Candidates: detect the gpt-oss
  family and assign a strategy; a final-message floor ("model produced
  no usable reply") when the completion is empty or is a bare
  tool-args object; Studio-side "(no reply)" rendering (already
  logged).
- **Verified fixed in the same re-test:** with the num_ctx budget and
  the tool-result cap deployed, per-call context estimates stay inside
  the 16k window (peak summed-turn input 43k across 5 calls ≈ 8.6k
  per call; no pruning storms, no server-side truncation).
- **P1 — pruning discarded the turn's own question. FIXED + verified.**
  The seven-turn workout's fat turn (9 tool calls, 18650→9580 prune)
  dropped the task message (oldest-first pruning; the question is the
  oldest turn-local message) — the model reset to a greeter reply and
  fired a rogue persona-flavored search (Colombian coffee), and the
  garbage final poisoned the next turn's replay. Fix: the pruner pins
  the task message, re-inserting it after the sentinel. Re-run: the
  same fat turn stays on-topic, the follow-up comparison answers
  properly, and the memory-vs-replay layering works (a turn-1 fact
  beyond replay depth correctly answered from Etch-written memory).
- **P3 (model quality, open) — identifier transposition.** With memory
  verified to hold "VH-EZT", the reply quoted "VH-EQT" — a one-letter
  flip on a rare token, stochastic (the previous run quoted it
  correctly). Candidate deterministic countermeasure for the polish
  waves: an identifier-fidelity check in the Candor family — flag a
  reply token that is edit-distance-1 from a recalled/tool-provided
  identifier (registrations, ICAO codes, part numbers).
- **P3 (model quality, open) — wrong sub-question under heavy fetch.**
  The pinned fat turn answered "is the C172 still in production?"
  instead of the asked cruise speed — on-topic now, but the model
  latches onto the page lead when the relevant section was truncated.
  Same gpt-oss finishing family as the empty completions.
- **P2 (agent behavior, open) — no currency check on source data.** The
  "GA airports near Perth" answer listed defunct 1930s aerodromes
  (Langley Park, Maylands, Caversham) read off a Wikipedia list that
  includes historical fields — and omitted Jandakot, the actual GA
  airport. Evidence discipline needs a "is this source current?"
  instinct; also feeds the never-invent P2 family.

### 2a · Daemon findings (from the same investigation, all fixed same-day)
- **P1 — reflection proposals were scope-dead on a clean v0.8.0.** The
  reflection scheduler's prompt instructs the model to call
  `reflection.propose`; a persona-delta-bearing call requires
  `persona.propose` — and the default-role floor granted **neither**
  (sixth registered-but-unauthorized floor gap). Audit chain: two
  ScopeDenied at the 07:44/08:17 boots; the model reported "my current
  permissions don't include the persona.propose capability". FIXED:
  both scopes added to the backcompat floor (governance-safe — a
  proposal only ever lands Pending behind the operator's approval).
- **P2 — reflection fired on EVERY daemon restart.** `last_fired`
  anchored at UNIX_EPOCH on boot, making the next fire always-past —
  four restarts this morning = four reflection turns (one burned the
  proposal attempt above). FIXED: anchor at boot time, so a restart
  waits for the next cron boundary, as the code's own comment always
  claimed.
- **P2 — "detected unclean shutdown" on every clean systemd stop.**
  The daemon only handled Ctrl-C (SIGINT); systemd stops with SIGTERM,
  which killed the process without dropping the StateGuard, leaving
  the crash-recovery state file behind. Every `systemctl restart`
  then reported a bogus unclean shutdown. FIXED: SIGTERM now cancels
  the shutdown token exactly like Ctrl-C.
- **P2 (observability, open) — web-chat turns are nearly invisible in
  the daemon journal.** Mission/cron turns log `→ tool {args}` /
  `[turn completed]` / final message; Studio chat turns log only the
  `recall:` line — the operator's ICAO mystery needed an audit-chain
  dump over IPC to reconstruct. Chat turns should journal at least
  tool calls + outcome like trigger turns do.

### 3 · Missions
- **P1 — a daemon restart zombifies the Studio silently. FIXED
  (657a3e6) after claiming its third operator action** (a mission Run,
  then a roster save + another Run during section 7). The page's /ws
  died with the daemon and the client ws task simply ENDED — stale
  signals kept every screen looking alive while all sends vanished.
  Fix: ws_task is a reconnect loop (1s/2s/4s/8s backoff), a sticky
  "Connection to the agent lost — reconnecting…" alert banner in the
  app shell, dashboard boot queries replayed on every (re)connect, the
  in-flight message a dying socket rejected is re-sent after
  reconnect, and the mission Run bar + roster Save gate on the
  connection signal like the chat composer. Dead sockets are detected
  within one poll tick.
- **P1 — Run gives zero feedback even on a live socket.** The client
  has no handler for `TeamRunStarted` and no `QueryError` arm for the
  `mc-start` id — success and failure are BOTH silently dropped; the
  row only ever appears via the background list poll. Fix: optimistic
  row / notice on `TeamRunStarted`, an error banner for `mc-start`
  errors, and an immediate `TeamMissionList` refresh.
- **P2 (team behavior) — specialists confabulate file-based handoffs.**
  Live mission (725be8d8): the writer opened its turn by reading
  `workspace: …/brief_text.md` — a file NO step ever wrote (the real
  handoff is `team.message` content) — got file-not-found in 17µs,
  quietly gave up, and the deliverable `conditions-brief.md` was never
  written. This is the 2026-07-03 "multi-step variance" item with a
  precise signature: the plan should state where each artifact LIVES,
  or member prompts should say "your inputs arrive in the message,
  not on disk."
- **Positives:** mission journaling is rich and readable
  (`aivyx team: [researcher] → web.fetch` per call); the artifact
  verdict honestly REJECTED the mission with a precise reason ("the
  one-line summary was stored in memory, but no conditions-brief.md
  exists"); member envelopes are correctly attenuated per role (the
  writer holds workspace scopes, the researcher doesn't).
- **P1 — the Reprise retry cap reset on every gate approval → infinite
  retry loop. FIXED.** The attempt counter was a driver-local, but an
  approval gate pauses a mission by RETURNING from `drive_registered`;
  each operator approval re-entered the function and reset the count.
  Live: five "attempt 2/2" retries, the same `gate_review_brief`
  prompt at the operator four times (with no attempt context shown —
  the gate label P3 stands). Fix: `verify_attempts` is persisted on
  the mission record.
- **P1 — the mission ended "done" with a confabulated deliverable: the
  verdict judge itself hallucinated a PASS.** Six consecutive honest
  rejections ("brief for KJFK instead of YPJT/YMML/YSSY", "placeholder
  text for Atlanta"), then the seventh sample ACCEPTED with a false
  rationale ("all required METARs were retrieved") over a file
  mentioning none of the three airports. gpt-oss:20b as judge ≈ ~15%
  false-PASS per sample here; unbounded retries made the false PASS
  inevitable. FIXED twice over: the retry cap bounds the samples, and
  a **deterministic identifier backstop** in the completion judge now
  rejects — before any LLM opinion, immune to fail-open — when the
  majority of the goal's uppercase identifiers (ICAO codes, tickers…)
  appear nowhere in the deliverable or evidence.
- **P1 — the whole team was structurally MCP-blind. FIXED.** No roster
  role declared an `mcp.call` base, so `bind_lead_scopes` filtered the
  daemon floor's per-server grants out of every specialist envelope,
  and the exact-name tool allowlists could never admit server-native
  MCP tool names anyway. The team spent 16 raw `web.fetch` calls
  approximating what one `get_flight_category` call returns — and the
  writer confabulated from the gaps. Fix: researcher/analyst/verifier
  declare an `mcp.call` marker; `filter_tools` expands it to bridged
  MCP tools by scope base; the binder flows the floor's QUALIFIED
  per-server grants and drops the bare marker (least privilege).
- **P2 (open) — the rejected-mission row shows no reason.** The
  operator saw REJECTED with no explanation; `halt_reason` carries the
  judge's precise verdict but the Missions row doesn't render it.
- **P2 (open) — inter-specialist handoff remains prompt-level fragile**
  even with tools fixed: the writer ignored the researcher's real
  Australian METAR data sitting in mission memory and wrote from
  priors. Plan prompts should state where each artifact lives; member
  prompts should say inputs arrive in the message.
- _(walkthrough note)_ the approval gate DID appear on the operator's
  own run (`gate_review_brief`, Approve/Reject rendered and worked);
  the earlier socket-launched run gated nothing — gate placement
  varies with the planned steps.
- **PROOF RUN (post-fix, same goal, 4th run):** specialists called
  `get_metar` on the aviation-weather MCP directly (no fetch thrash);
  attempt 1's placeholder was honestly rejected; attempt 2 produced a
  brief with REAL YPJT/YMML/YSSY METARs; the judge rejected on genuine
  defects and — the fixed assertion — **the mission terminated after
  exactly 2 attempts.** Bounded, honest, tool-equipped. A separate
  identifier-free goal (flat-white essay) passed its verdict, proving
  the deterministic backstop stays out of the way when the goal names
  nothing.
- **P3 (model quality) — METAR field transposition in the brief:**
  `22012KT` (220° at 12 kt) rendered as "220 kt" in the wind column —
  the identifier/precision family again, now in structured-data
  reading. Feeds the same Candor-style fidelity-check candidate.
- **Positive — abort:** `aivyx team abort <id>` on an executing
  mission halted it at the next step boundary; phase flipped to
  `halted` with `halt_reason: "aborted by operator"` persisted and the
  journal line matching. **Section 3 complete** — open items feed the
  polish waves: zombie-page reconnect/banner, Run feedback, rejection
  reason on the row, gate labels with attempt context, handoff
  fidelity prompts.

### 4 · Memory / Wiki / Graph
- **Positives (operator verdict 2026-07-05):** memory screen renders the
  accumulated mission/chat content well ("lots of info saved"), search
  works as expected, data is correct throughout, no internal-topic
  leaks (pre-flight confirmed data-side too).
- **P3 — the graph views are "messy and unintuitive."** Data correct,
  presentation not: the operator wants layout/readability work
  (clustering, label collision, visual hierarchy). Joins the Command
  Center restyle as the Studio polish wave's design workload.
- **P2 (data-side, from the same session) — mission turns have no
  topic-naming discipline.** Specialists filed single entries under
  bare-ICAO topics (`YPJT`, `YMML`, `YSSY`) alongside
  `overall_conditions` + `overall_conditions_summary` — three naming
  conventions from one mission. Candidate: a topic-naming hint in the
  member/mission prompts, or a mission-scoped topic prefix.
- **P2 — contradictory memory entries carry no on-screen indication.**
  `overall_conditions` holds 6 entries from the mission retries,
  including a direct contradiction ("No METAR data available" vs "All
  three Australian airports are VFR"). Concord detects conflicts but
  only the CLI (`aivyx memory conflicts`) surfaces them — the Memory
  screen should badge conflicted topics.
- _(watch-item)_ wiki page keyed `operator-note` (singular) vs live
  memory topic `operator-notes` (plural) — verify backlinks connect;
  if not, the Codex topic-keying has a normalization gap.

### 5 · Skills (Repertoire)
- **Positives (operator verdict):** all 6 skills render with correct
  data (triggers, procedures, provenance); zero-state effectiveness
  displays. UI needs the same refinement pass as the other screens.
- **P2 (self-learning, the section's headline) — skill USE has a
  cold-start problem on a fresh agent, and the whole Whetstone arc
  starves behind it.** Live probes (2026-07-05): a turn matching
  `summarize-document`'s trigger word-for-word did the work directly —
  no `skills.invoke`, no SkillInvocation event — and even NAMING the
  skill explicitly didn't elicit an invocation from gpt-oss:20b.
  Mechanism: skills surface via the LEARNED tool-relevance ledger
  (empty on day one) and model initiative (local models don't take
  the indirection). Old Jarvis's organic Whetstone proof rode days of
  warmed history. Without invocations there are no effectiveness
  samples, no Candor-graded skill turns, no correction retro-folds —
  refinement can never start. Candidate fix (strong): **semantic
  trigger matching** — embed skill triggers, match against each turn's
  query like recall does, and inject the top skill's procedure
  directly into turn context; skill "use" becomes structural instead
  of an indirection the model must choose, and injected-skill turns
  can be graded. (Charter-nudge and ledger-warming are weaker
  fallbacks.) **BUILT + LIVE-PROVEN same-day (2e1af39 + 4b13d51):**
  `SkillTriggerContext` matches skill triggers per turn (embedding
  cosine with cached trigger vectors; token-overlap fallback for
  embedding-free installs) and injects the top match's procedure as a
  labeled, capped, header-defanged block composed with recall in the
  planner's context slot. Threshold calibrated live on
  nomic-embed-text: floor 0.50 (true match 0.68; a briefing-adjacent
  false positive at 0.47 correctly excluded on re-test). `[skills]
  trigger_injection = false` opts out. Follow-up LANDED same-day
  (3e8ca41) after the operator hit it live ("skill used but the screen
  says otherwise"): the turn id now reaches the injection seam
  (TurnPlanner::begin_turn + ContextProvider::recall carry TurnId) and
  every injection emits a turn-correlated `SkillInvocation` inside the
  turn's audit range. Live proof: one injected summarize turn →
  audit-chain SkillInvocation + Repertoire `invocations: 1, samples: 1`
  — Whetstone folded its FIRST effectiveness sample from an injected
  use. The full arc (inject → record → count → grade) is closed.
- **COMPREHENSIVE SKILLS CHECK (2026-07-05, post-fix battery):** all 6
  skills + a taught 7th verified live. Positives: `draft-reply` 0.60,
  `daily-briefing` 0.72, `research-and-summarize` 0.57,
  `suggest-next-steps` 0.72; negatives (arithmetic, raw METAR ask)
  stay silent at the 0.50 floor; an ambiguous multi-intent goal
  injected exactly ONE skill (top-1 discipline). **Teach→use proven
  end-to-end**: `aivyx skills teach metar-decode …` was live on the
  next turn with no restart, injected at 0.76, and the reply followed
  the taught procedure's format. **Update→use proven** after the check
  caught two more bugs, both fixed same-day (a237e44 + 2055272):
  (1) the trigger-embedding cache keyed `name@version`, but Tutor
  updates preserve version — a rewritten trigger served its STALE
  embedding forever; now keyed on the embedded text
  (self-invalidating); (2) `capture-note`'s type-level trigger
  ("shares a fact worth remembering") embeds nowhere near concrete
  instances ("my medical expires 15 March 2027") — rewritten with
  instance nouns in the Outfit defaults + on the rig chain. Post-fix:
  the medical-certificate fact injected `capture-note` at 0.55 and
  the agent SAVED IT to memory with a one-line confirmation — **which
  structurally closes the persist half of the §2 volunteered-fact
  P2**. Residual (minor): "key points of README.md" picked the
  adjacent `research-and-summarize` (0.56) over `summarize-document`
  — top-1 cosine between sibling skills is fuzzy; harmless while
  procedures overlap.
- **Creation paths staged for the overnight tick:** [skill_authoring]
  (Praxis) runs at reflection cadence (02:30 post-fix) and the
  aviation topics are exactly its knowledge-rich + skill-less input;
  the per-turn auto-proposer evaluated today's turns (heuristic
  signals partially matched, nothing filed yet). Verify tomorrow:
  pending proposals should show Praxis output — the first organic
  proposal test since the floor fix made proposals grantable at all.

### 6 · Agents (proposal triage — live governance test)
_(pending)_

### 7 · Teams
- **Positives (operator verdict):** all 9 Nonagon members render with
  appropriate data and settings (roles, souls, tools, scopes — incl.
  the new `mcp.call` markers); same UI-polish-only verdict as the
  other screens.
- **The "lost roster save" + "missions broken again" reports were both
  the §3 zombie page** (the v0.8.1 release deploy restarted the daemon
  under the operator's open tab; reads rendered cached state, writes
  vanished). Root-caused via journal silence + no team file on disk;
  fixed by the reconnect work above.
- **Save verified post-fix:** the writer-soul edit round-tripped
  byte-perfect into the conventional `~/team.toml` (first write on
  this install), the save confirmation showed, and a mission launched
  from the same fresh socket. Restart-required semantics noted: the
  running mission uses the boot-time roster; edits drive missions
  after the next daemon restart. **Section 7 complete** — findings are
  UI-polish only beyond the (fixed) zombie page.

### 8 · Documents
- **Positives (operator verdict):** browsing, reading, and editing all
  work; "promising." Document CONTENT is ~95% accurate with some
  invented details — that's the agent-authorship confabulation family
  (§2b/§3), not a browser bug; the browser renders faithfully what the
  agent wrote.
- **P2 — editor forced single-line text. FIXED (0bca834):**
  `.doc-edit` used `white-space: pre`; now wraps like the read view.
- **P2 — saved edits looked stale until a screen reload. FIXED
  (0bca834):** Save now re-reads the file through the ordered ws
  bridge so post-write content refreshes in place, and a refresh
  re-list no longer closes an open viewer out from under an edit.
- **P3 (polish wave) — render markdown as markdown.** Operator wants
  .md files formatted (headings, tables, lists) with mermaid support
  rather than raw monospace text — a natural fit since agent
  deliverables are markdown; pairs with the UI modernization pass.
- _(milestone note)_ the 0bca834 deploy doubled as the reconnect fix's
  first live proof: the operator's open page rode a daemon restart
  with the banner + self-heal instead of zombifying.

### 9 · Settings
- **Positives:** read view matches the running config; the write path
  PASSED the surgical-write test — a two-knob edit (autonomy level →
  unleashed, deliberate; a no-op budget policy line) diffed to exactly
  those lines, with the hand-maintained token/MCP/comment content
  byte-identical (the toml_edit promise held on a real, hand-edited
  config).
- **P2 (product) — settings coverage is thin.** Operator verdict: the
  screen exposes a limited subset of the config surface; he wants a
  future chapter to inventory `aivyx.toml`'s operator-relevant knobs
  and expose them properly ("more settings for the end user"). Queue
  as a v0.9 chapter candidate alongside the UI modernization pass.
- **P3 (safety UX) — autonomy-tier knobs deserve a louder confirm.**
  The level change to unleashed saved as casually as any field; the
  operator confirmed it was deliberate here, but a dial that composes
  the agent's entire permission posture should get an explicit,
  unmissable confirmation (the destructive-action pattern), and
  arguably a chip on the Command Center showing the pending-restart
  divergence.

### 10 · MCP (Lantern) + Voice
- **MCP — P2 (product, the operator's headline): the screen is
  read-only.** No add, edit, update, or remove — MCP servers are
  manageable only by hand-editing `[[mcp_server]]` in `aivyx.toml` +
  restart. Operator wants full lifecycle management in the Studio.
  Chapter candidate: the established write-half recipe (shared
  toml_edit writer + server-side validation + restart-required UX,
  proven by Settings/Teams/Roster) applied to `[[mcp_server]]`,
  including `env`/`headers` with `${VAR}` interpolation (Conduit) and
  ideally a "test connection" probe before save. Pairs with §9's
  settings-coverage chapter — both are config-write surface area.
- **MCP — data-side pre-flight was healthy** (both servers connected,
  3 tools each, clean stderr). Known-limitation note stands: the
  status is connection-level from the last daemon start — web-search
  showed green all day while DuckDuckGo refused its queries; a
  tool-level health signal (recent success/failure from the audit
  chain) is the polish-wave candidate.

- **Voice — empty state PASSES** (setup instructions render; no
  config on the rig). **Operator decision: voice work is deferred
  until after v1.0** — core surfaces take priority.

### 11 · `/classic` — panes worth porting vs deleting
- Inventory (2026-07-05, against the live Studio): of the ten
  /classic panes — chat, missions, audit, sessions, profile, persona,
  proposals, notifications, memory, learning — six are fully covered
  by Studio screens (chat/missions/memory directly; profile/persona/
  proposals by Agents). **Four need porting before retirement:**
  1. **audit** — the Studio has only the Command Center tail; the
     full-chain browser (pagination, event types) has no equivalent.
  2. **sessions** — no Studio equivalent.
  3. **notifications** — history exists over IPC (Phase 73) but no
     Studio screen renders it.
  4. **learning** — the GetLearningInsights view (reflection cadence,
     recall feedback) has no Studio home; candidate: fold into the
     Command Center rather than a dedicated screen.
  All four fit the read-only screen recipe. Retirement keeps the
  minimal no-bundle fallback page per the plan (item 7 is NOT a blind
  delete).

## TUI + desktop shell
_(pending — separate pass)_

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
  cannot see that it just asked a question.
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
_(pending)_

### 4 · Memory / Wiki / Graph
_(pending)_

### 5 · Skills (Repertoire)
_(pending)_

### 6 · Agents (proposal triage — live governance test)
_(pending)_

### 7 · Teams
_(pending)_

### 8 · Documents
_(pending)_

### 9 · Settings
_(pending)_

### 10 · MCP (Lantern) + Voice
_(pending)_

### 11 · `/classic` — panes worth porting vs deleting
_(pending)_

## TUI + desktop shell
_(pending — separate pass)_

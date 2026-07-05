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
  candidate answer and bias the memory hook accordingly).
- **P3 (agent behavior) — 3 identical memory.search calls in one turn**
  before concluding absence; sits right at the Bridle repeat-call
  threshold. One search (or search + list-topics) should suffice.

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

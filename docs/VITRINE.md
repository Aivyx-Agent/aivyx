# Through the Shop Window — the operator walkthrough (Chapter Vitrine)

> **Status: PAUSED (2026-07-04) — rig hardware failure before the walkthrough began; resumes after the rig rebuild.** The v0.9 polish backlog, captured
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

### 1 · Command Center
_(pending)_

### 2 · Chat
_(pending)_

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

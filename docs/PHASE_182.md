# Phase 182 — Guided In-Agent Credential Onboarding

**Chapter H, phase 3.** The backend review's Tier-2 gap: the
Chapter F/G productivity tools (Gmail, Calendar, Drive, …) work,
but *connecting* one means knowing an undocumented sequence —
create a Google Cloud OAuth app, hand-write
`$HOME/.aivyx/tool-processes/<service>/config.toml` with the
client id/secret, run a *separate* per-service binary's
`auth init`, then wire `[[tool_process]]` into `aivyx.toml`.
Phase 182 replaces that with one guided command: **`aivyx
connect <service>`**.

## What exists (and what's actually missing)

The OAuth *dance* already works well. Each Google service binary
(`aivyx-gmail` / `aivyx-calendar` / `aivyx-drive`) has an
`auth init` that builds the consent URL, runs a **loopback
auto-catch** server, exchanges the code, and writes `tokens.json`.
The friction is entirely *upstream and around* that flow:

1. Knowing the sequence exists at all.
2. Creating the Google Cloud OAuth app + copying client id/secret.
3. Hand-writing `config.toml` before `auth init` will run.
4. Using a separate `aivyx-<service>` binary, not the main one.
5. Wiring `[[tool_process]]` afterward.

So Phase 182 is a **guided wrapper**, not a re-implementation —
it reuses the tested per-service `auth init` by shelling out to
it (the operator-chosen approach), and fills in everything
around it.

## The flow — `aivyx connect [service]`

- **`aivyx connect`** — lists the connectable Google services
  with their status (connected / not), so discovery is built in.
- **`aivyx connect gmail`** — the guided onboarding:
  1. **Already-connected check.** If `tokens.json` exists, offer
     to re-connect or stop.
  2. **Google Cloud app guidance.** Clear, numbered steps —
     enable the API, create an OAuth client ID of type *Desktop
     app*, copy the client id + secret — with the console link.
     (We guide; Google's console is outside our control.)
  3. **Collect credentials.** Paste prompts for client id +
     secret.
  4. **Write `config.toml`.** To
     `$HOME/.aivyx/tool-processes/<service>/config.toml` with the
     id/secret, the loopback `redirect_uri`, the per-service
     default scopes, and the token path — `0600`.
  5. **Resolve the service binary.** From an existing
     `[[tool_process]]` `command` in `aivyx.toml` (by name) →
     else a sibling of the running `aivyx` binary → else a
     prompt.
  6. **Run the auth flow.** Shell out to `<binary> auth init`
     with inherited stdio, so the consent URL, browser, and
     loopback catch happen live; a timeout bounds a stuck
     consent.
  7. **Confirm.** Verify via `<binary> auth status` /
     `tokens.json`, and report success warmly.
  8. **Offer to wire `[[tool_process]]`.** If no enabled entry
     exists for the service, offer to append/enable one in
     `aivyx.toml` (via `toml_edit`, preserving the rest).
- **Conversational surfacing.** When a Google tool process is
  configured but unauthenticated (no `tokens.json`) at daemon
  startup, the breadcrumb + the tool-unavailable guidance name
  the exact fix: *"run `aivyx connect <service>`."*

## Design principles

- **Reuse, don't re-implement.** The OAuth dance stays in the
  per-service `auth init`; `aivyx connect` shells out to it. The
  per-service flows are unchanged and keep working standalone.
- **Operator-verified where it must be.** The dance + browser +
  Google can't run in CI; that boundary is verified on the
  operator's machine (the threat-model / sandbox pattern). The
  wrapper's *pure* pieces — the service registry, `config.toml`
  render, binary-path resolution, the `[[tool_process]]` append
  — are unit-tested.
- **Privacy / local-first preserved.** Credentials are the
  operator's own OAuth app, stored only in the per-tool-process
  `config.toml` / `tokens.json` (outside the daemon store, per
  the threat model). No Aivyx server in the path.
- **Google-first, by choice.** API-key services (Notion / n8n —
  paste a token) are a lighter follow-on; this phase nails the
  high-friction loopback flow.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 181's frozen hash (`fc8eafe`).
2. **Service registry + `config.toml`.** A small registry
   (gmail / calendar / drive → display name, binary name, default
   scopes, console hint, redirect port) + the `config.toml`
   renderer/writer + the already-connected check. Tests: registry
   lookup, config render round-trips into the service's expected
   shape, connected-status detection.
3. **`aivyx connect` command + guided flow.** CLI parse
   (`connect`, `connect <service>`), the guided collection
   (guidance → paste creds → write config → resolve binary →
   shell out to `auth init` → confirm). Tests: CLI parse,
   binary-path resolution precedence, scripted credential
   collection (the shell-out itself is operator-verified).
4. **Auto-wire `[[tool_process]]` + surfacing.** Offer to
   append/enable the tool in `aivyx.toml` (`toml_edit`); the
   daemon-startup "unauthenticated → run `aivyx connect`"
   guidance. Tests: the append/enable helper, the surfacing
   message.
5. **INSTALL + exit + Frozen.** INSTALL section (the connect
   flow, the Google Cloud setup, the API-key follow-on note);
   exit doc; README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 18 → **19**. A new
  operator-facing CLI command + config writing over the *existing*
  Chapter F credential substrate — no new capability scope, no
  new core tool (the 13-tool cap is untouched), no new
  `KeyDomain`, no contract change.
- **PRODUCT.md** — **Will hold.** Streak: 72 → **73**. Makes the
  P10/P12 third-party-tool onboarding usable; not a new
  commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 18 →
  **19**. Work lands in the `aivyx` binary (`aivyx-channel`) +
  `aivyx-config`; `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_182.md` + README row + Phase 181 backfill — T1.
- [ ] Service registry + `config.toml` write + connected check — T2.
- [ ] `aivyx connect [service]` guided flow (shell-out) — T3.
- [ ] `[[tool_process]]` auto-wire + startup surfacing — T4.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies (`std::process` + `toml_edit`
  + the existing google-oauth/auth-cli substrate).
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+20`. *(Dense components: the
  registry + the config round-trip + the `toml_edit` append +
  path-resolution precedence + scripted collection. The shell-out
  flow is operator-verified, not unit-tested — no Google in CI.)*

## Honest scope risks at sign-off

- **The OAuth dance + browser are operator-verified.** Shelling
  out to `auth init` (loopback, consent, Google) can't run in CI;
  the wrapper's pure pieces are tested, the live flow is verified
  on the operator's host.
- **Requires the service binary present.** Shell-out depends on
  `aivyx-<service>` being installed (it's the tool the operator
  runs anyway); resolution falls back to a prompt, and a missing
  binary is a clear error, not a crash.
- **Google Cloud app creation is guided, not automated** — the
  console is outside our control; we give exact steps + the link.
- **API-key services (Notion / n8n) are deferred** to a follow-on
  (the operator-chosen Google-first scope).

## Prediction vs reality

_(Filled at exit.)_

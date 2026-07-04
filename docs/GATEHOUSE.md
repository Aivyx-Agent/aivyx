# Two Keys at the Door — the exposure interlock (Chapter Gatehouse)

> **Status: COMPLETE (GH.0–GH.1, 2026-07-04).** v1.0-runway decision 2,
> locked 2026-07-04. Chapter Postern already built the Studio's auth
> *mechanism* (a `[daemon] web_ui_auth_token` checked constant-time at
> the `/ws` control plane and the page load: Bearer / HTTP Basic /
> cookie) but chose warn-don't-refuse for the unauthenticated off-host
> bind. Gatehouse turns that warning into a **two-key launch**: an
> unauthenticated agent with filesystem and shell reach can never be
> exposed to a network *by accident* — the ~175k-exposed-Ollama-hosts
> lesson, applied before v1.0's appliance story invites it.

## What shipped (GH.1)

- **The interlock, enforced at config load** (not at bind time — a
  bind-task failure would log-and-limp; a config error stops the daemon
  with the resolution block): `web_ui_host` beyond loopback with no
  `web_ui_auth_token` is a `ConfigError` naming both remedies, unless
  the operator explicitly signs the risk with the new
  `[daemon] web_ui_insecure_no_auth = true` (the
  behind-my-own-authenticating-reverse-proxy escape hatch — TLS and
  fancier auth legitimately remain proxy jobs). Loopback installs are
  byte-identical.
- **The Harbor appliance generates its token at first boot**: the
  entrypoint inserts a 43-char alphanumeric token (256-bit, URL-safe by
  construction) into the seeded config's `[daemon]` section and prints
  it once to the container log — the appliance stays
  works-out-of-the-box under the interlock instead of refusing to
  start. Operator-set tokens and the escape hatch are respected
  (idempotent, guarded on both keys' absence).
- Four-quadrant config tests (refused / hatch / token / loopback) + an
  offline test of the entrypoint insertion.

## Deliberately not here

- No accounts, sessions, or OIDC — Nexus/Passport-era.
- No token generation in `aivyx daemon install` for native installs:
  they are loopback-default, the interlock only bites when an operator
  configures exposure, and the config error at that moment names the
  exact remedy. Add it only if operator friction shows up in practice.
- The Studio needs no token prompt of its own — Postern's HTTP Basic
  page-load prompt plants the cookie the `/ws` upgrade carries.

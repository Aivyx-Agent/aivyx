# Containerized Deployment (Chapter Harbor)

> **Status:** 🟡 **design contract** (Chapter Harbor opening, HB.0). The locked
> contract for shipping Aivyx as a **docker-compose appliance**: a one-command,
> always-on daemon + Studio that a user runs on a homelab box or VPS, without a
> Rust toolchain or a native binary on their main machine. This document fixes
> the **framing, scope, and the two security-sensitive daemon changes** the
> deployment requires, before any `Dockerfile` is written.

## 1. The gap — there is no "just run it" server deployment

Today Aivyx installs as a native binary (the cargo-dist [shell installer](INSTALL.md#shell-installer-recommended))
or builds from source. Both put the daemon on the operator's own machine, which
is right for the **desktop, local-first** experience. There is **no packaged way
to stand up Aivyx as an always-on service** — the homelab / VPS / "give me an
assistant at `studio.mybox.lan`" deployment — without manually installing the
binary, the tool processes, a config, and a process supervisor on the host.

`docker compose up` is the idiomatic answer for that audience. It is also the
natural home for the **headless execution mode** already on the roadmap
([`HEADLESS_MODE.md`](HEADLESS_MODE.md)): a non-interactive, gates-don't-block
daemon *is* a containerized daemon. The two reinforce each other.

## 2. The key insight — this is a deployment *profile*, not a second product

The single most important decision in this chapter is **what Harbor is not**.

Aivyx's core promise is **local-first**: the agent reaches *your* files, *your*
devices, *your* network, and your key talks straight to the LLM. A container
inverts most of that — "the filesystem" is the container's, "localhost" is the
container's, there are no audio devices. So Harbor is **explicitly the
server-appliance profile**, not a drop-in for the native desktop install:

| Native install (desktop, local-first) | → Harbor (server appliance) |
|---|---|
| Reaches the host's real files (`access` home/full) | Reaches a **bind-mounted data volume** (`/work`) — intentional, scoped |
| OAuth via a host browser + loopback | OAuth via a **documented published-port recipe** |
| Voice (host audio devices) | **No voice** (no `/dev/snd` in the profile) |
| Interactive passphrase prompt | **`AIVYX_PASSPHRASE` via a Docker secret** |
| Studio on `127.0.0.1:7843` for the local user | Studio exposed deliberately, **opt-in**, with auth/TLS in front |

Pitching Harbor as "the same thing, in Docker" would half-defeat the local-first
model and mislead users. Pitching it as "Aivyx-as-an-appliance" is honest and
genuinely valuable. **The docs lead with that distinction.**

## 3. What already works (more container-ready than expected)

- **Non-interactive unlock exists.** The daemon sources the store passphrase from
  `AIVYX_PASSPHRASE` (`aivyx-channel::passphrase`, `DEFAULT_ENV_VAR`) before
  falling back to the interactive prompt. A detached container unlocks with **no
  code change** — supplied as a **Docker secret / file**, not a bare env var
  (§6).
- **Tool processes are just binaries.** Bake `aivyx-gmail` / `aivyx-calendar` /
  `aivyx-drive` / `aivyx-contacts` / `aivyx-notion` / `aivyx-obsidian` /
  `aivyx-n8n` / `aivyx-toolkit` into the image and the `[[tool_process]]`
  `command` entries resolve — no host PATH setup for the user.
- **musl static builds exist.** cargo-dist already produces musl artifacts
  (Chapter Q), so the runtime image can be **distroless / `scratch`** — small,
  fast to pull, minimal attack surface.
- **The config + state is one directory.** Everything lives under `~/.aivyx/`
  (config, encrypted store, per-tool tokens) → a single named volume.

## 4. The two REQUIRED daemon changes (both opt-in, security-sensitive)

The web UI server is **deliberately localhost-only**, which collides with Docker
networking. Harbor needs two small daemon changes — both **default-off**, so
every existing native install behaves identically.

### 4.1 Configurable web bind host (`web_ui.rs:158`)

`run_web_ui_server` binds `SocketAddr::from(([127, 0, 0, 1], port))` — hardcoded
loopback. Docker port-publishing forwards host:port → the **container's
`0.0.0.0`**, never its loopback, so a 127.0.0.1-bound process is **unreachable
even with `-p 7843:7843`**. Harbor adds an opt-in bind host (e.g.
`[daemon] web_ui_host = "0.0.0.0"`, default `127.0.0.1`). **Required even for the
localhost-only spike** (the container must bind `0.0.0.0` for the forward to land).

### 4.2 Configurable WS Origin allowlist (`web_ui.rs:~269`)

The `/ws` upgrade enforces an Origin allowlist hardcoded to `http://127.0.0.1:<port>`
and `http://localhost:<port>` (the CSWSH / DNS-rebinding defense — keep it). When
Studio is published to **host** localhost on the same port, the browser still
sends `Origin: http://localhost:7843`, which **passes** — so a localhost-only
spike needs *only* §4.1. But accessing from another machine
(`http://studio.mybox.lan:7843`) sends a non-allowlisted Origin and is
**rejected**. Remote access therefore needs an opt-in allowlist
(`[daemon] web_ui_allowed_origins = ["https://studio.mybox.lan"]`), empty by
default (= localhost-only, today's behavior). Loosening it is a real exposure;
the docs pair it with "put auth + TLS in front."

> **Why this is in the contract.** Harbor is not just packaging — it touches the
> daemon's network-exposure security posture. Locking these two opt-in knobs
> (defaults unchanged) *before* coding is the whole point of writing this first.

## 5. The frictions, and how Harbor handles each

1. **Filesystem reach.** `access` home/full means "reach the host's files"; in a
   container it means "reach the bind-mounted `/work` volume." For an appliance
   working in a data volume this is correct and even desirable. The docs set
   `access` to a workspace rooted at `/work` and explain the boundary. *Not a
   bug — a reframing.*
2. **OAuth loopback.** `aivyx connect` opens a **host** browser to
   `127.0.0.1:<port>/callback`, but that loopback is the container's. Resolution:
   publish the connect callback port, register the **host-mapped** `redirect_uri`
   in the Google app, run the consent from the host browser. A documented recipe
   (HB.3), not a code change. The fiddliest part of onboarding — called out
   plainly.
3. **Web exposure.** Covered by §4. Default localhost-only; remote is opt-in with
   auth/TLS guidance.
4. **Local models (Ollama).** Bundling Ollama as a **sibling compose service** is
   a zero-setup local-LLM on-ramp (`http://ollama:11434` over the compose
   network). **GPU passthrough** (the nvidia container runtime) adds host setup,
   so it's an **optional profile**; CPU-only works out of the box (slow). Users
   with Ollama already on the host point at `host.docker.internal:11434` instead.
5. **Voice — excluded.** Needs host audio devices; INSTALL.md already notes
   containers don't do audio. Harbor is daemon + Studio + tools (+ optional
   Ollama). Voice stays a native host CLI.
6. **Passphrase posture.** `AIVYX_PASSPHRASE` works but a passphrase in a plain
   env var is weaker than an interactive prompt (visible in `docker inspect` /
   process env). Harbor uses a **Docker secret mounted as a file** and documents
   the tradeoff honestly — it is the appliance security posture, a deliberate
   step down from the desktop prompt.

## 6. The minimal compose shape (target of HB.1)

```yaml
services:
  aivyx:
    image: ghcr.io/aivyx-agent/aivyx:latest      # HB.5 publishes this
    ports: ["127.0.0.1:7843:7843"]               # host-localhost only by default
    volumes:
      - aivyx-data:/root/.aivyx                   # config + encrypted store + tokens
      - ./workspace:/work                         # the agent's fs_root (access level)
    environment:
      - AIVYX_PROVIDER=anthropic                  # cloud profile first (no GPU/OAuth)
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
    secrets: [aivyx_passphrase]                   # → AIVYX_PASSPHRASE via file
    # config (baked or mounted) sets: [daemon] web_ui_host="0.0.0.0", web_ui=true,
    #   [access] level rooted at /work
  # ── optional local-model profile ──
  ollama:
    image: ollama/ollama:latest
    profiles: ["ollama"]
    volumes: [ollama-data:/root/.ollama]
volumes: { aivyx-data: , ollama-data: }
secrets: { aivyx_passphrase: { file: ./secrets/passphrase } }
```

The runtime image is multi-stage: a musl/static build stage → a distroless
runtime carrying the `aivyx` daemon **plus every tool-process binary**, with the
embedded Studio bundle (already in the daemon binary). `CMD` runs the daemon
(`aivyx daemon run`).

## 7. What's deliberately *not* here

- **Not a Kubernetes chart / Helm.** One box, one compose file. K8s is a later
  call if demand appears.
- **Not voice, not host-device access** (§5).
- **Not a managed/multi-tenant service.** Single-operator appliance; no built-in
  user management. Multi-user is a product question, not this chapter.
- **Not auth/TLS termination.** Harbor documents putting a reverse proxy
  (Caddy/Traefik) in front for remote exposure; it does not ship one.
- **Not a desktop-install replacement** (§2).

## 8. Phase plan

| Phase | Deliverable |
|---|---|
| **HB.0** | This contract. |
| **HB.1** | The **cloud-provider spike**: multi-stage `Dockerfile` (musl/static → distroless, daemon + all tool binaries) + `docker-compose.yml` for the Anthropic/OpenAI profile (no Ollama, no OAuth). Includes the §4.1 `web_ui_host` change (opt-in). Acceptance: `docker compose up` → Studio reachable at `http://localhost:7843`, a chat turn completes, state persists across `down`/`up`. |
| **HB.2** | The §4.2 opt-in **Origin allowlist** (`web_ui_allowed_origins`, default empty = localhost-only) + the optional **Ollama sibling** service (CPU profile out of the box; documented GPU profile). |
| **HB.3** | The **OAuth-in-Docker recipe** (§5.2) — published callback port + host-mapped `redirect_uri`, verified end-to-end against one Google service. Doc, not code. |
| **HB.4** | **Docs**: this file flipped to shipped + an INSTALL.md "Docker" section leading with the appliance-vs-desktop framing (§2), the passphrase-secret + exposure/TLS guidance, and the worked compose. |
| **HB.5** | **CI image publish**: a workflow that builds + pushes `ghcr.io/aivyx-agent/aivyx` on each version tag (alongside the existing cargo-dist binaries). |

## 9. Open questions (resolved at HB.N)

- **F-1 (HB.1):** does the image **bake a default `aivyx.toml`** (appliance
  defaults: `web_ui_host=0.0.0.0`, `access` at `/work`, tool processes wired) and
  let a mounted config override it, or require the user to supply one? *Lean:
  bake a sensible appliance default; mount-to-override. Zero-config `up` is the
  whole value proposition.*
- **F-2 (HB.1):** distroless vs `debian-slim` runtime. *Lean: distroless static
  (musl) for size/surface; fall back to `debian-slim` if a tool process needs
  glibc/dynamic deps (audit at HB.1).*
- **F-3 (HB.2):** Ollama GPU profile — ship the nvidia-runtime compose override
  in-repo, or document it only? *Lean: document + a commented `compose.gpu.yml`
  override; don't make GPU a default path.*
- **F-4 (HB.4):** how hard to push users toward a reverse proxy for any non-
  localhost exposure — a hard warning in the daemon logs when `web_ui_host` is
  non-loopback **and** the allowlist is non-empty, or docs only? *Lean: a
  one-line startup warning; cheap, and exposure is the main footgun.*

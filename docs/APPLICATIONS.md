# Using your open applications (Chapter Deckhand)

**Opt-in. Default off.** Aivyx PA can, when you turn it on, use the GUI
applications already open on your own machine — list windows, focus one,
type / press keys / click, and capture a screenshot. This gives the agent
"hands" on your desktop for the workflows that have no CLI or API.

It is deliberately **off by default and tightly fenced**: driving live GUI apps
reaches your whole desktop and cannot be sandboxed, so it is the opposite of the
auditable, scoped tool calls Aivyx PA is built on. The capability is therefore
Trusted-tier only, confirm-first on input, and every action is a named,
audited tool call.

## Enabling it

```toml
[applications]
enabled = true
# binary_path = "aivyx-apps"   # optional; default is `aivyx-apps` on PATH
```

This synthesizes the `aivyx-apps` tool process (run **unsandboxed** on purpose —
GUI control needs the host display) and grants the default agent the `app.*`
scopes. Restart the daemon after changing it.

### Required host tools (Linux)

v1 shells out to standard Linux utilities (no extra Rust dependencies):

- **`xdotool`** — window list / focus / keyboard / mouse (`pacman -S xdotool`,
  `apt install xdotool`).
- A screenshot CLI for `app.screenshot` — one of **`grim`** (Wayland),
  **`maim`**, **`scrot`**, or ImageMagick **`import`**.

A missing tool produces a clear install hint at call time, not a crash.

## The tools

| Tool | Scope | Notes |
|---|---|---|
| `app.list` | `app.read` | `{windows: [{id, title, active}]}` — `id` feeds `app.focus` |
| `app.screenshot` | `app.read` | full-screen PNG → `{path, tool}`; interpret with a vision model |
| `app.focus` | `app.control` | raise/focus a window (reversible) |
| `app.type` | `app.input` | type literal text into the focused window — **confirm-first** |
| `app.key` | `app.input` | send a key/chord (`"ctrl+s"`, `"Return"`) — **confirm-first** |
| `app.click` | `app.input` | click at `(x, y)` — **confirm-first** |

## Trust model

- **Trusted-tier only.** All three bases (`app.read`, `app.control`, `app.input`)
  are in `CEILING_TRUSTED` and absent from `CEILING_SEMITRUSTED`: a remote /
  SemiTrusted channel adapter can never hold them.
- **`app.input` is confirm-first.** It is in `IRREVERSIBLE_BASES`, so with
  `[access] confirm_destructive` on, every keystroke/click injection pauses for
  your approval (the same gate as `fs.delete` / `git.commit`).
- **Audited.** Each `app.*` call is a normal tool call on the HMAC audit chain.
- **Access level.** Intended for an agent you run at `home`/`full` access on your
  own machine — not a remote-exposed deployment.

## Platform limitation (read this)

v1 targets **Linux X11 + Xwayland** via `xdotool`. **Native Wayland windows are
isolated by the compositor and are not reachable** — `app.list` won't see them
and input won't reach them. Apps running under Xwayland (most Electron/X11 apps)
work. macOS (Accessibility / AppleScript) and Windows (UI Automation) are future
backends.

## Verifying it

This capability cannot be tested on a headless server (the dogfood rig is
TTY-only) — it needs a real desktop session. On your desktop, after enabling it:

1. `aivyx-pa doctor` / start the daemon; confirm the `applications` tool process is
   listed.
2. Ask the agent to "list my open windows" → `app.list` returns them.
3. Ask it to focus one and type into it → `app.focus` then `app.type` (approve
   the confirm-first prompt).

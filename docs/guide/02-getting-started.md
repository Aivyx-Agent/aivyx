# Getting started

This page gets you from a fresh install to a running assistant.

## 1. Install

Pick the path that matches your machine — the full instructions live in the
[install guide](https://github.com/Aivyx-Agent/aivyx/blob/main/docs/INSTALL.md):

- **macOS / Linux** — run the one-line installer, or build from source.
- **Windows** — run Aivyx under WSL2, or use the Docker appliance. (There is no
  native Windows build yet; both options run the same Linux binary.)
- **Always-on server** — the Docker appliance runs the daemon and this Studio in
  a container.

## 2. Choose how your assistant thinks

Aivyx needs an AI model to reason with. You have two kinds of choice:

- **Local model (free, private, no API key).** Install [Ollama](https://ollama.ai)
  and pull a tool-capable model — `qwen3:8b` is the recommended starter. Aivyx
  detects Ollama automatically and sizes everything for you. Nothing leaves your
  machine.
- **A provider (Anthropic or OpenAI).** Paste an API key during setup. These
  models are more capable; you pay the provider per use.

You can switch later — this is just your starting point.

## 3. First launch

Run the setup wizard once:

```sh
aivyx init
```

It detects your model, then walks you through **creating your agent** — giving
your assistant a name, a personality, and a level of access to your machine.
That flow is covered in detail on the next page.

When the wizard finishes it writes a small config file (`aivyx.toml`) and you're
ready. Launch the assistant with:

```sh
aivyx
```

This starts the daemon (if it isn't already running), drops you into a chat
session, and serves this Studio at **http://127.0.0.1:7843**.

## 4. Open the Studio

Visit **http://127.0.0.1:7843** in your browser. The first time, you'll land on
the **Create your agent** screen if you haven't set up an identity yet;
otherwise you arrive at the **Command Center** dashboard.

> **Prefer a real app?** The [desktop app](11-desktop-app.md) puts the Studio in
> its own window and your assistant in the system tray — with approval
> notifications and a summon hotkey — instead of a browser tab.

## Health check

If anything seems off — an empty first reply, a model that won't connect — run:

```sh
aivyx doctor
```

It checks your model, your config, and a live test reply, and tells you exactly
what to fix. See [Troubleshooting](10-troubleshooting.md) for the common cases.

# Installing Aivyx

This doc covers the full install matrix. For the abbreviated
"Five-minute setup" path, see the [root README](../README.md).

Aivyx ships a single binary, `aivyx`, plus three optional channel
adapters baked into it (CLI, Telegram, Web UI). There are no
hosted dependencies — your binary talks directly to your LLM
provider (Anthropic / OpenAI-compatible / Ollama) and stores
everything locally in an encrypted redb file.

## Current install state

Phase 61 wired the release pipeline (cargo-dist, GitHub Actions
CI, four-target matrix, install-script generation) but did
**not** publish a release. Until public hosting is configured
and Task 7 lands, the only supported install path is
[build from source](#build-from-source-currently-the-only-path).

The shell-installer section below documents the path that will
become primary once `v0.1.0` is published.

## Supported targets

When the release pipeline fires, it will cover four targets. All
Linux builds are musl-static, so a single Linux binary works on
every distro without glibc version drift.

| Target | Binary | Notes |
|---|---|---|
| Linux x86_64 (musl) | `aivyx` | Debian 8+ / Ubuntu 16+ / Arch / Alpine / RHEL 7+ |
| Linux aarch64 (musl) | `aivyx` | ARM64 servers, Raspberry Pi 4/5 (64-bit OS), Asahi Linux |
| macOS x86_64 | `aivyx` | Intel Macs, macOS 10.13+ |
| macOS aarch64 | `aivyx` | Apple Silicon (M1 / M2 / M3 / M4), macOS 11+ |

Native Windows is **not yet supported**. The daemon's IPC layer
uses Unix domain sockets (mode 0600, OS-user identity); a Windows
port needs a NamedPipe replacement. Until that lands, run Aivyx
inside [WSL2](https://learn.microsoft.com/en-us/windows/wsl/) — it
behaves as a regular Linux x86_64 install.

## Build from source (currently the only path)

This is the supported install path today. Cargo build from a
clone of the repository:

**Prerequisites:**
- Rust toolchain 1.85+ (`rustup` recommended)
- A C linker (`gcc` / `clang` / Xcode CLT)
- For Linux musl builds: the `musl-tools` package (Debian) or
  equivalent

```sh
git clone https://github.com/AivyxDev/aivyx
cd aivyx
cargo build --release --bin aivyx
# binary lands at target/release/aivyx
```

Install into `~/.cargo/bin/` (if `cargo install` is preferred):

```sh
cargo install --path crates/aivyx-channel --bin aivyx
```

The pre-commit hook (`./scripts/install-hooks.sh`) is optional
for end users; it enforces `cargo clippy --workspace --all-targets
-- -D warnings` on every commit and is recommended for
contributors.

## Shell installer (when published)

This section documents the path that becomes primary once
`v0.1.0` is published. **It does not work yet** — the URL
returns 404. The pipeline is in place to fire on the first tag
push to a public GitHub remote.

The cargo-dist-generated installer will detect your arch,
download the right tarball, verify its checksum, and drop
`aivyx` into `$CARGO_HOME/bin/` (typically `~/.cargo/bin/`).

```sh
# Will work post-publication:
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/AivyxDev/aivyx/releases/latest/download/aivyx-channel-installer.sh \
  | sh
aivyx --version
# aivyx 0.1.0
```

For a specific version, replace `latest` with the tag:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/AivyxDev/aivyx/releases/download/v0.1.0/aivyx-channel-installer.sh \
  | sh
```

### macOS first launch: Gatekeeper

Phase 61's release will not include code signing or notarization.
macOS quarantines unsigned binaries downloaded from the network.
Two ways past the warning:

**(a) Strip the quarantine attribute** (one-shot, recommended):

```sh
xattr -d com.apple.quarantine "$(command -v aivyx)"
```

**(b) Right-click → Open** the binary once from Finder. macOS
asks for confirmation; after that, future invocations work.

Signing + notarization is on the deferred-distribution list. It
requires an Apple Developer account and a CI-side cert pipeline;
it lands in a follow-up phase once operator pressure surfaces.

## Where files land

| File | Default location | Configurable? |
|---|---|---|
| `aivyx` binary | `~/.cargo/bin/aivyx` | Yes — `--install-path` flag on the installer |
| Config | `./aivyx.toml` (CWD) or `~/.config/aivyx/aivyx.toml` | Yes — `--config <path>` on `aivyx`; the wizard writes to CWD by default |
| Encrypted store | per-config (`[storage] path`) | Yes — TOML `[storage] path` |
| Daemon socket | `$XDG_RUNTIME_DIR/aivyx.sock` (Linux) / `$TMPDIR/aivyx.sock` (macOS) | No |
| Daemon PID file | `$XDG_RUNTIME_DIR/aivyx.pid` (Linux) / `$TMPDIR/aivyx.pid` (macOS) | No |
| Web UI port | `127.0.0.1:7843` | Yes — TOML `[daemon] web_ui_port` or `--web-ui-port <N>` |

## First-run checklist

After install:

1. **`aivyx init`** — interactive wizard. Detects Ollama at
   `http://127.0.0.1:11434` and offers it as the default
   provider (no API key required). Otherwise prompts for an
   Anthropic or OpenAI key. Writes `aivyx.toml` to your CWD with
   `0600` permissions. Three optional Profile prompts (assistant
   name, primary use case, communication style) seed your
   operator identity layer.

2. **`aivyx`** — auto-spawns the daemon (foreground or
   background depending on flag), drops you into a REPL session,
   and serves the Web UI on `127.0.0.1:7843` if you enabled it.

3. **Visit `http://127.0.0.1:7843/`** for the Web UI: Chat,
   Missions, Audit (with cold-verify), Sessions, Profile,
   Persona tabs.

4. **`aivyx --verify-only`** at any time runs the offline
   HMAC audit-chain verification pass.

For deployment guidance (threat model, what Aivyx defends
against, what it doesn't), read
[`docs/THREAT_MODEL.md`](THREAT_MODEL.md) before exposing the
agent to anything sensitive.

## Moving Aivyx to a new machine

Phase 64 ships **identity export**: a portable snapshot of your
operator-declared Profile and the reflection-approved Persona
chain. Useful for backup before risky changes, for migrating
between machines (laptop → VPS, old host → new host), or for
inspecting the chain offline with `jq`.

```sh
# On the source host (daemon must be running):
aivyx identity export ~/aivyx-snapshot.json
# Wrote N deltas + Profile to ~/aivyx-snapshot.json
# File permissions: 0600 (owner-only).
```

The file is pretty-printed JSON with the following shape:

```json
{
  "schema_version": 1,
  "exported_at": "2026-05-14T14:30:00Z",
  "source_host": "laptop.local",
  "profile": { "assistant_name": "...", "..." : "..." },
  "persona": {
    "deltas": [ { "seq": 0, "delta": { "..." : "..." } }, ... ],
    "effective_at_export": { "..." : "..." }
  }
}
```

The chain's HMAC MACs are deliberately omitted from the export
— the per-host HMAC key is not portable. On import the chain
is re-signed with the target host's key. Trust comes from
operator authority, not cross-host cryptographic provenance.

To restore a snapshot on a target host (Phase 65):

```sh
# On the target host (daemon must be running):
aivyx identity import ~/aivyx-snapshot.json

# If the target host already has a Persona chain, the import
# refuses by default to avoid silent overwrite. Pass --force
# to wipe and replace:
aivyx identity import ~/aivyx-snapshot.json --force
```

On success the daemon refreshes its runtime persona state
immediately — the next agent turn sees the imported persona
without a restart.

**Profile import is operator-driven.** The export bundle
includes the source host's `[profile]` section for reference,
but `aivyx identity import` does not auto-write `aivyx.toml`.
To apply the imported Profile, hand-edit the target host's
`aivyx.toml` to match the bundle's `profile` block, then
`aivyx daemon stop && aivyx` to reload. This keeps the
destructive-write scope tight to one on-disk artifact (the
encrypted Persona chain).

## Uninstall

```sh
# Remove the binary
rm "$(command -v aivyx)"

# Stop and remove the daemon socket/PID (if a daemon is still around)
aivyx daemon stop 2>/dev/null
rm -f /run/user/$UID/aivyx.sock /run/user/$UID/aivyx.pid

# Remove the encrypted store and config (DESTROYS YOUR DATA)
rm -f ./aivyx.toml /tmp/aivyx-store.redb
# Adjust paths to match your config's [storage] path.
```

The encrypted store contains your conversation history, audit
chain, memory, missions, schedules, profile, and persona deltas.
Deleting it is irreversible — there is no cloud backup by
design (PRODUCT.md G6).

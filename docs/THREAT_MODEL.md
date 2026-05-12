# Aivyx Threat Model

**Status:** Draft. **Last reviewed:** Phase 46 exit (2026-05-10).
**Owners:** the operator.

This document is **operator-facing**. It states plainly what Aivyx
defends against, what it does not, and where each defense lives in
the code. It is a sibling of `DESIGN.md` (technical contract) and
`PRODUCT.md` (product contract), not a derivation of them — those
two documents describe *how the agent works*; this document
describes *what an operator can and cannot rely on it for*.

If a claim in this document disagrees with the code, the code is
right and the document is wrong — file an issue.

---

## 1. Scope

Aivyx is a **single-operator personal agent**. The threat model is
written around exactly one human, on hardware they control, talking
to LLM providers under their own API key, holding secrets they
own.

This is the same posture OpenClaw and Hermes Agent take. The model
is **not**:

- a multi-tenant SaaS,
- a shared workstation tool where two humans take turns,
- a server-side bot answering anonymous web traffic.

Operators who run Aivyx in a context that breaks the single-operator
assumption (e.g., a shared dev box where a second user can `read(2)`
the IPC socket) are responsible for understanding that the model no
longer applies.

## 2. The operator and their adversaries

The operator's identity is **the OS user who owns the daemon
process** (`PRODUCT.md` P6). There is no Aivyx-level account, no
password, no token. If you can read the daemon's IPC socket
(mode `0600`, owner = operator UID), you are by definition the
operator. Rotation of an "Aivyx account" is therefore not a
concept; rotation of the redb passphrase is.

The model recognizes four **adversary archetypes**, aligned with
the trust tiers in `aivyx-capability/src/lib.rs:503` (D5):

| Tier | Archetype | Example | Default authority |
|---|---|---|---|
| `Kernel` | Aivyx itself | the turn loop, audit writer | unconditional (internal use only) |
| `Trusted` | the operator at their own keyboard | Local CLI, Web UI on `127.0.0.1` | near-total, with extra audit on destructive ops |
| `SemiTrusted` | the operator over a remote, authenticated channel | their own Telegram bot, with chat-id allowlisted | narrowed — no unqualified shell, no `fs.delete`, qualifier required on `fs.*` and `net.post` |
| `Untrusted` | anyone the operator has not authenticated | webhook requests, unallowlisted senders | near-empty — read public memory, that's it |

A turn loop computes the **effective capability set** exactly
once per turn, before any LLM call:

```rust
let effective = self.capabilities().intersect(tier.default_ceiling());
```

This snapshot is recorded in the `TurnStarted` audit event
(`aivyx-audit/src/lib.rs:91`) and is authoritative for the
entire turn. Mid-turn capability escalation is not supported.

## 3. Assets

What an attacker would gain by compromising each.

| Asset | Where it lives | If compromised |
|---|---|---|
| Passphrase | In RAM during cold start; never on disk | Full read/write of the encrypted store. |
| Master key | `MasterKey` in daemon RAM, zeroize-on-drop (`aivyx-crypto/src/lib.rs:175`) | Same. |
| Encrypted store | `$XDG_DATA_HOME/aivyx/store.redb`, chmod 0600 | Confidential without the passphrase; needs Argon2id work to brute. |
| Audit chain | redb `KeyDomain::Audit` | Reading reveals every tool call ever made. Tampering trips `AuditError::ChainBroken` on next open. |
| API keys (LLM provider, Telegram bot token) | `KeyDomain::Secrets`, AEAD-sealed under a domain subkey | Spend on operator's LLM account; impersonate operator's bot. |
| Memory entries | `KeyDomain::Memory`, AEAD-sealed under a domain subkey | Reveals everything the operator told the agent across sessions. |
| Daemon IPC socket | `$XDG_RUNTIME_DIR/aivyx/aivyx.sock`, mode 0600 | Anything the operator can do. |
| Source code & config | `~/Projects/.../aivyx/`, `aivyx.toml` | Loosen role envelopes, add malicious tools. |

Nine encrypted domains exist today (`aivyx-storage/src/lib.rs:117`):
Sessions, Memory, Audit, Secrets, ChannelState, Missions,
Schedules, Webhooks, FileWatches. Each is sealed under its own
HKDF-derived subkey so a leak of one domain's plaintext does not
compromise another.

## 4. Threats we defend against

This section names each threat, the mitigation, and the code that
implements it. Anything not on this list is in section 5.

### 4.1 An LLM is steered into running a destructive command

**Example.** A prompt-injection payload in a web page or a malicious
upstream MCP-server tool description steers the LLM into emitting
`shell.exec("rm -rf $HOME")`.

**Mitigation chain:**

1. **Capability scope check** before execution. The tool's
   `required_scope(input)` is checked against the turn's effective
   capability set (`aivyx-core/src/agent.rs`). A `SemiTrusted`
   turn does not hold `shell.exec` at all
   (`CEILING_SEMITRUSTED`, `aivyx-capability/src/lib.rs:627`).
2. **Per-role allowlist.** Even at `Trusted`, if the active role's
   `tool_allowlist` omits `shell.exec`, the synthetic
   `tool.allowlist:<tool>` scope is denied
   (`aivyx-capability/src/lib.rs:65`).
3. **Process-group isolation.** The shell subprocess is launched in
   its own process group; SIGTERM→SIGKILL escalation on timeout
   prevents zombie grandchildren (Phase 42).
4. **Environment isolation.** `shell.exec` strips API keys and
   provider tokens from the child's environment so an LLM-generated
   command cannot exfiltrate them via `echo $ANTHROPIC_API_KEY`
   (Phase 42).
5. **Audit trail.** The denied call (or, if it ran, the call +
   output hash) lands in the HMAC-chained audit log
   synchronously. There is no path that runs a tool without
   appending an audit row first.

### 4.2 The on-disk store is read by a process that is not the daemon

**Mitigation:** ChaCha20-Poly1305 AEAD under a subkey derived from
the operator's passphrase via Argon2id (m=64 MiB, t=3, p=4) and
HKDF-SHA256 with a versioned salt (`aivyx-crypto/src/lib.rs:58`).
The store file is `chmod 0600` on every cold open
(`aivyx-storage/src/lib.rs:471`).

**Caveats.** Argon2id parameters are tunable. The salt is versioned
(`aivyx-v1-storage`) so a future key-schedule migration can produce
entirely different subkeys from the same master.

### 4.3 The audit log is tampered with on disk

**Mitigation:** Every `SignedEntry` carries an HMAC-SHA256 tag over
`prev_mac || canonical_bytes(event)` (`aivyx-audit/src/lib.rs:164`).
The genesis seed is `b"aivyx-audit-v1-genesis"`. Canonicalization
is JCS (RFC 8785) via `serde_jcs`. The HMAC key is itself an HKDF
subkey under `KeyDomain::Audit`. Tampering with any byte of any
entry breaks the chain at the first modified row and trips
`AuditError::ChainBroken` on the next open.

`aivyx --verify-only` cold-verifies the full chain without an LLM
API key, so audit verification works on a machine that has never
been online.

### 4.4 A process on the same machine tries to talk to the daemon

**Mitigation:** The daemon's IPC socket is a Unix domain socket
under `$XDG_RUNTIME_DIR` with mode `0600`, owner = operator UID
(`aivyx-channel/src/daemon_server.rs:177`). There is no network
surface. There is no token exchange. If you can `read(2)` the
socket, you are the operator by OS-level identity (`PRODUCT.md` P6).

**Caveats.** This assumes the runtime directory is also private to
the operator (true under standard systemd-logind setups). On a box
where another OS user has `read` on `$XDG_RUNTIME_DIR/aivyx/`, the
model breaks — but that already required compromising the
operator's user account.

### 4.5 The passphrase is captured from memory

**Mitigation, partial:** The transient passphrase buffer is wiped
via `zeroize::Zeroize` immediately after Argon2id derivation
(`aivyx-channel/src/passphrase.rs`). `MasterKey` and `SubKey` are
`ZeroizeOnDrop` (`aivyx-crypto/src/lib.rs`) and have no public API
that hands out raw bytes — callers get `seal`/`open` methods.
`Debug` is redacted.

**Caveats.** A core dump, a debugger attached to the running
daemon, or a memory-scraping rootkit will defeat this. We do not
defend against an attacker with kernel-level access to the
operator's machine.

### 4.6 A SemiTrusted channel tries to act with Trusted authority

**Example.** An attacker controls a Telegram chat that the operator
allowlisted. They ask the agent to `cat ~/.ssh/id_rsa`.

**Mitigation:** `CEILING_SEMITRUSTED` does not include `shell.exec`,
`fs.delete`, `config.write`, or unqualified `fs.read` /
`fs.write`. The turn-loop intersection (`effective = agent_caps ∩
tier_ceiling`) is computed *before* the LLM sees the message
(`aivyx-capability/src/lib.rs:516`). The agent literally cannot
emit a passing tool call for these scopes from a SemiTrusted turn.

### 4.7 A scheduled run, webhook, or file watcher executes with too much authority

**Mitigation:** Trigger-launched turns inherit the OS user's
identity and run under a configured role's envelope, not under
`Kernel`. Webhook triggers bind to `127.0.0.1` only
(`aivyx-channel/src/webhook_listener.rs`). The webhook source is
classified as a channel and given a tier — `Untrusted` by default.
Triggered missions can opt into `wrap_mission = true` so every
triggered run lands in the mission audit surface.

### 4.8 Reflection / self-modification runs without operator oversight

**Mitigation:** The reflection loop is three audited steps
(Phases 28–30):

1. `turn.history` — agent reads its own recent outcomes (audited as
   a regular tool call).
2. `reflection.propose` — agent writes a proposed change to a gate
   queue. **The change is not applied.** A `mission.gate` audit
   event names the operator-approval requirement.
3. `reflection.apply` — runs only after the operator answers the
   gate (CLI prompt, Telegram `/approve` command, Web UI button).
   The mutation lands in the audit chain.

The agent has no path to silent self-modification. Both `memory`
and runtime role overrides (`RoleOverrides`, Phase 30) flow through
the same approval gate.

## 5. Threats we explicitly do not defend against

The honest section. These are out-of-scope by design; if they
matter to your deployment, you need additional controls *outside*
Aivyx.

### 5.1 The operator's machine being root-compromised

If an attacker has the operator's UID or kernel access, every
in-RAM key, every plaintext memory entry, and every audit row is
theirs. The threat model assumes the OS underneath Aivyx is sound.

### 5.2 A malicious MCP server

Aivyx ships MCP support (Phases 23/24/32). MCP servers run as
**child processes of the daemon**, under the operator's UID, with
read access to everything the operator can read. There is **no
signed-server registry, no sandboxing, no content-level scan of the
server binary or its descriptor payload**.

Tool calls into an MCP server are gated by capability scopes
(`mcp.call:<server>:<tool>`), so an MCP tool that asks for
`shell.exec` does not silently get it. But the server *process
itself* runs with operator authority and can read whatever a
shell command can. Treat each MCP server install with the same
caution as installing a CLI tool from a stranger's tarball.

### 5.3 Prompt injection beyond capability gating

Aivyx has no content-level scanner for prompt-injection payloads in
fetched web pages, in MCP tool descriptions, or in
operator-provided context files. Hermes Agent ships Tirith and
context-file scanning; Aivyx does not. The defense available today
is **only** capability gating: an LLM that has been jail-broken
into trying to exfiltrate data still cannot emit a passing tool
call for a scope its role does not hold.

This is intentional within a known limitation: capability gating
is a stronger boundary than pattern-matching scanners, but it does
not catch attacks that stay within the agent's *legitimate*
authority (e.g., a prompt that convinces the agent to read a file
it is allowed to read and post it to a URL it is allowed to post
to). Operators are responsible for not granting roles authority
they would regret if the LLM acted maliciously.

### 5.4 Network-level eavesdropping on LLM provider traffic

Aivyx uses `rustls` over HTTPS for every LLM provider call. We
trust the TLS stack and the operator's CA roots. If a corporate
or hostile MITM has injected a root CA into the operator's trust
store, the agent will use it.

### 5.5 The LLM provider itself being malicious

API keys go straight to Anthropic / OpenAI / Ollama. We do not
defend against an LLM provider exfiltrating prompt content,
returning poisoned tool calls, or correlating the operator's API
usage. The privacy guarantee is *not* "the cloud cannot see your
prompts" — it is *"only the provider you chose can see your
prompts, with the API key you supplied, under your account"*
(`PRODUCT.md` G6, N5).

### 5.6 Tools running in the same address space as the daemon

`PRODUCT.md` P12 commits to tool-process IPC isolation, but that
work has not shipped. Every tool today (including third-party MCP
proxies) runs **in the daemon's process**. A buffer overflow in a
tool can in principle corrupt daemon memory.

`#![forbid(unsafe_code)]` in `aivyx-crypto` and the absence of
unsafe blocks elsewhere mean this is harder than in C, but it is
not impossible. P12 is the long-term mitigation; until it lands,
tool authors are inside the daemon's trust boundary.

### 5.7 Side channels (timing, power, electromagnetic)

We use constant-time AEAD primitives from RustCrypto. We do not
defend against an attacker who can measure the daemon's wall-clock
behavior or power draw. This is appropriate for a personal agent;
operators in adversarial environments (red-team training labs,
nation-state targets) should not rely on Aivyx for this.

### 5.8 Channel-platform compromise

If Telegram is compromised, the operator's bot token leaks and an
attacker can send messages as the operator. Aivyx will then
classify them as `SemiTrusted` (because the chat-id allowlist
matches) and run them within the tier ceiling. The damage is
bounded by `CEILING_SEMITRUSTED`, but it is not zero. This is the
trade for using third-party messaging platforms at all.

### 5.9 Adversarial co-tenants

A second OS user on the same machine who has been granted access
to the operator's home directory, runtime directory, or store file
defeats the OS-level identity model. The fix is the OS's job:
don't share UIDs. Aivyx does not enforce isolation between OS
users of the same instance because — per `PRODUCT.md` P1 — there
is no such thing as a multi-tenant Aivyx instance.

## 6. Property summary

For operators asking "what should I be able to assume about a
running Aivyx daemon":

1. **Confidentiality at rest:** Yes, against anyone without the
   passphrase. AEAD + Argon2id.
2. **Tamper-evidence of the audit chain:** Yes. HMAC-SHA256 chain,
   genesis-seeded, JCS-canonical, cold-verifiable offline.
3. **Authority bounding by tier:** Yes. The agent cannot exceed
   the tier ceiling for a turn, full stop.
4. **Authority bounding by role:** Yes. The operator's role config
   (`aivyx.toml`) attenuates further per the single-inheritance
   tree (`PRODUCT.md` P7, P9).
5. **No silent self-modification:** Yes. Reflection writes go
   through operator-approved gates.
6. **No hosted control plane:** Yes. The operator's API key talks
   directly to the model provider; storage stays on the operator's
   hardware (`PRODUCT.md` G6, N5).
7. **Container-level sandboxing of tools:** **No.** Out-of-process
   tool isolation (P12) is forward work.
8. **Prompt-injection content scanning:** **No.** Operators are
   responsible for what they grant a role authority to do.
9. **Defense against a compromised OS user:** **No.** Outside scope.

## 7. Reporting a vulnerability

If you find a way to break any "Yes" in section 6, or any defense
in section 4, please open an issue or contact the maintainers
privately. Do not publish exploits against the audit chain, the
storage cipher, or the capability check before the maintainers
have had a chance to ship a fix; the operator population is small
enough that responsible disclosure makes a real difference.

## 8. Document discipline

This file is a draft until reviewed alongside `DESIGN.md` D4 + D5
and `PRODUCT.md` P1 + P6 + P10 + P12. When it lands as
non-draft, treat it the way the phase journals treat their
contract documents: edits go through a phase commit so drift is
visible. Section 4 grows when new defenses ship; section 5 shrinks
when forward commitments deliver (notably P12).

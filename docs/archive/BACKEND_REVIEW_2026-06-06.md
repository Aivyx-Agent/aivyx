# Aivyx Backend Review — What We Have vs. Gaps to the Goal

**Date:** 2026-06-06 (Phase 179 exit)
**Scope:** Backend infrastructure only. The dedicated **frontend is
explicitly excluded** — the current `:7843` Web UI is a minimal
built-in, and the channel adapters (CLI / voice / Telegram /
Discord / Slack) are the present interface surface.

> **Update (Phase 184 exit).** The Tier-1 + Tier-2 gaps this
> review named were closed by **Chapter H (Phases 180–184)** — a
> default sandbox (180), a guided first-launch identity builder
> (181), guided `aivyx connect` credential onboarding (182),
> reminders (183, everyday-PA breadth #1), and conversational
> skill-teaching (184). The remaining open items are the Tier-3
> bookends: cutting `v0.1.0` (external hosting), the
> everyday-PA breadth continuation, the dep-requiring hardenings,
> and the Channel Activation Milestone. See
> [`ROADMAP.md`](../ROADMAP.md) Chapter H.

**Founding goal.** A **Local-First, Security-Focused, fully
customizable autonomous Personal Assistant** that an End User
shapes by **Personality + Role** at first launch and uses for
*any* purpose — a secure local alternative to Openclaw / Hermes
Agents.

**Codebase as reviewed:** 24 crates, ~199k LOC of Rust, 4,139
tests passing, 0 clippy warnings, 179 phases shipped, 13 contract
amendments.

> This is a point-in-time reference, not a contract. For the
> locked contracts see [`../DESIGN.md`](../../DESIGN.md) /
> [`../PRODUCT.md`](../../PRODUCT.md); for the phase narrative see
> [`ROADMAP.md`](../ROADMAP.md); for the prior whole-agent review
> see [`AGENT_REVIEW_2026-06-05.md`](AGENT_REVIEW_2026-06-05.md).

---

## Part 1 — What We Currently Have

### ✅ Pillar 1: Local-First — strong, essentially complete

| Capability | Implementation | Local? |
|---|---|---|
| Chat inference | `aivyx-llm`: **`mistral_rs`** (in-process GGUF), **ollama**, any OpenAI-compatible local server | ✅ fully local |
| Cloud option | Anthropic + OpenAI providers | optional |
| Embeddings (memory recall) | OpenAI-*compatible* endpoint → ollama / llama.cpp / text-embeddings-inference | ✅ local-capable |
| Voice | `aivyx-voice`: **piper** (TTS) + **whisper / whisper-rs** (STT) | ✅ fully local |
| Storage | `aivyx-storage`: redb, on-disk, encrypted | ✅ local |
| Hosted dependency | **None in the request path** | ✅ |

An operator can run chat, embeddings, *and* voice with zero cloud
calls. Cloud is opt-in, not assumed.

### ✅ Pillar 2: Security-Focused — strong, the load-bearing differentiator

- **Capability model** (`aivyx-capability`): 70 scope bases,
  `CapabilitySet`, 4-tier `TrustTier`
  (Kernel/Trusted/SemiTrusted/Untrusted) with per-tier capability
  **ceilings** and attenuation rules. Every tool call is
  capability-checked.
- **Audit** (`aivyx-audit`): HMAC-chained, append-only,
  **offline-verifiable**; tampering trips `ChainBroken`.
- **Crypto** (`aivyx-crypto`): Argon2id → HKDF-SHA256 →
  ChaCha20-Poly1305, zeroize-on-drop master key.
- **Encryption at rest**: 19 HKDF-isolated storage domains, one
  subkey each.
- **Trust tiers per channel**: remote channels are
  SemiTrusted/Untrusted by default; gated escalation.
- **OAuth isolation**: productivity-tool tokens live in
  **per-tool-process files**, not the daemon store — a daemon
  compromise does not reach them.
- **Documented threat model** ([`THREAT_MODEL.md`](../THREAT_MODEL.md))
  with explicit in-scope / out-of-scope.

### ✅ Pillar 3: Customizable by Personality + Role at first launch — present, the core identity model

- **`init` wizard** (`init.rs` + templates): collects
  `profile_assistant_name`, `profile_primary_use_case`,
  `profile_communication_style`; three bundled archetypes
  (**coder / researcher / personal**) pre-fill the wizard.
- **Profile (P13)** — operator-declared identity
  (`Profile { operator_profile, … }`) injected into the system
  prompt.
- **Persona (P14)** — reflection-*written* character that evolves
  via gated proposals (the "Soul").
- **Roles** (`roles: BTreeMap<String, Role>`) — per-role
  capability envelopes (`tool_allowlist`, `memory_topic_prefix`),
  runtime-switchable via `role.switch`.

### ✅ Pillar 4: Autonomous — strong and recently deepened

- **Missions** (`mission.rs`):
  `Created → Running → GatePending → Completed/Failed/Cancelled`
  state machine with operator gates.
- **Triggers**: cron schedules, **webhooks** (127.0.0.1),
  **file-watchers** — each launches a turn under a role envelope.
- **Reflection loop**: self-improvement with operator-gated
  `reflection.propose` / `apply`.
- **The Aivyx Ralph loop** (Phases 173–177): fully autonomous,
  self-re-arming agent over an HMAC-chained backlog, with
  iteration / wall-clock / token caps, driver-side gate
  verification, and a cross-iteration progress log.
- **Proactive outreach**: `notify_dispatcher`
  (email/telegram/webhook/webui) + `proactive_detect` — the agent
  reaches *out*, not just responds.

### ✅ Pillar 5: "Any purpose" general assistant — broad, extensible

- **13-tool substrate core** (`fs.*`, `shell.exec`,
  `web.{fetch,post}`, `git.*`, `net.dns`, `memory.*`,
  `role.switch`, `skills.*`) — DESIGN A12 cap.
- **Productivity integrations** (each a sandboxed OAuth tool
  process): Gmail, Calendar, Drive, Notion, Obsidian, n8n, plus
  the `aivyx-toolkit` bundle (web.search, task.*, health.check.*).
- **Extensibility**: MCP client (`aivyx-mcp`), the tool-process
  SDK (any language), the channel SDK, and a **skills** system
  (composable user-defined capabilities + auto-proposer).
- **Self-learning differentiator**: recall-feedback tuning,
  helpfulness / co-occurrence ledgers, and the correction-signal
  loop (structural → LLM-judged → tool-attributed). *This is the
  real edge over Openclaw / Hermes.*

---

## Part 2 — Gaps to Fully Fulfill the Goal

Ranked by how load-bearing each is to the stated goal.

### 🔴 Tier 1 — Blocks the goal as stated

| Gap | Detail | Why it matters |
|---|---|---|
| **No default sandbox** | The sandbox (`aivyx-tool`) is a *wrapper* — the operator must supply a bubblewrap / firejail / Docker policy. Out-of-the-box, tool processes run with the operator's full UID. | For a "security-focused" PA aimed at non-expert end users, an unconfigured default means the security posture is not on by default. A bundled default policy (or a "secure mode" preset) closes this. |
| **Distribution is dormant** | Release pipeline wired but unpublished; install path is build-from-source. | "An End User launches their new Aivyx Agent" implies a downloadable artifact. Cutting `v0.1.0` is the missing step. |
| **First-launch personality is thin** | The wizard collects 3 free-text fields + a template. There is no guided, LLM-assisted personality/role *builder*. | The goal emphasizes launch-time identity generation as the defining UX. Functional but minimal for "fully customizable at launch." |

### 🟠 Tier 2 — Meaningfully narrows "any purpose"

| Gap | Detail |
|---|---|
| **Common PA domains absent** | No contacts/CRM, weather, maps/location, messaging/SMS, finance/expense tracking (roadmap notes budget tracking as *future*), smart-home, or media control. The covered set skews developer / knowledge-worker (Gmail / Calendar / Drive / Notion / n8n). |
| **No credential/secrets UX for tools** | Secrets live in `KeyDomain::Secrets`, but onboarding a new OAuth productivity tool is a CLI/manual flow (`aivyx-auth-cli`), not a guided in-agent step. |
| **Skills are code-shaped** | The skills system is powerful but author-facing; an end user cannot *teach* the agent a new skill conversationally yet. |

### 🟡 Tier 3 — Polish / known carry-overs (from the roster)

- Cryptographic PRNG for jitter (currently `SystemTime`-seeded)
  and full-compression PDF page counting — both deferred to avoid
  breaking the zero-new-dependency streak.
- Tool-level **rate limiting / quota** enforcement (capability
  gates *what*, not *how often*).
- The **Channel Activation Milestone** (68 deferrals) —
  cross-channel session continuity so the agent feels like *one*
  assistant across Local / voice / Discord / Slack / Telegram.
  The single largest unrealized item; directly serves the
  "personal assistant everywhere" promise.
- Only ~32 `TODO`/deferred markers across the two largest crates —
  the backend is genuinely mature; these are refinements, not
  holes.

### ⚪ Explicitly out of scope (per the review instruction)

- The **dedicated frontend**. The current `:7843` Web UI is a
  minimal built-in; channel adapters are the present interface
  surface.

---

## Bottom line

The backend **substantially realizes four of the five pillars**
(Local-First, Security-Focused, Autonomous, general-purpose) and
has a *genuine* differentiator over Openclaw / Hermes in the
self-learning correction loop. The goal's gaps concentrate in
three places:

1. **Security-by-default for non-expert users** — a bundled
   sandbox preset so the posture is on without operator setup.
2. **The last mile of distribution + a richer first-launch
   identity builder** — cut `v0.1.0`; make the
   personality/role step a guided builder.
3. **Breadth of everyday-PA tool domains + cross-channel
   continuity** — fill the common-PA gaps and land the Channel
   Activation Milestone so it feels like *one* assistant.

None of these are architectural holes — they are the expected
"productize it" layer on top of a mature, disciplined substrate.

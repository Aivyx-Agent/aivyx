# Aivyx Backend Framework — Audit Review

**Date:** 2026-06-16 (v0.2.0, post-Studio)
**Scope:** The agent framework **backend** — all 32 workspace crates
(~230K LOC). The Studio web frontend is in scope only where it is a
new *backend* attack surface (the `/ws` bridge and the config/filesystem
write paths it now exposes).
**Method:** Full sweep of tech-debt markers, `unsafe`, panics, production
`unwrap`/`expect` density, the crypto / capability / audit security core,
the new web write surfaces, dependency vulnerabilities (`cargo-audit`),
and the status of every gap named by the prior reviews. Cross-checked
against the 4,668-test suite.

> Point-in-time reference, not a contract. For the locked contracts see
> [`../DESIGN.md`](../DESIGN.md) / [`../PRODUCT.md`](../PRODUCT.md); for the
> phase narrative see [`ROADMAP.md`](ROADMAP.md); for the security model see
> [`THREAT_MODEL.md`](THREAT_MODEL.md). Supersedes the archived
> [`AGENT_REVIEW_2026-06-05`](archive/AGENT_REVIEW_2026-06-05.md) and
> [`BACKEND_REVIEW_2026-06-06`](archive/BACKEND_REVIEW_2026-06-06.md)
> (Phase 179/184), whose blocking gaps are now closed (§2).

---

## 1. Headline — the framework is mature and clean

| Signal | Result |
|---|---|
| Real TODO/FIXME debt | **~0** — the 9 hits are `XXXXX` OAuth-doc placeholders + the literal `task.*` todo-store |
| `todo!()` / `unimplemented!()` | **0** |
| Production panics | **0** (all 107 `panic!` are test assertions / test error-handlers) |
| `unsafe` blocks | **41, all benign** — edition-2024 `env::set_var` (mostly tests) + a few `libc::kill`/`killpg` for process mgmt |
| Prod `unwrap`/`expect` in `crypto` + `storage` | **0** (the security core) |
| Dependency CVEs (`cargo-audit`) | **0** |
| Ignored tests | **0** |
| Tests passing | **4,668**, 0 clippy warnings |

No architectural holes; no critical security defect in the core
(crypto, capability, audit, storage). The disciplined-substrate
reputation holds up under inspection.

## 2. Prior-review gaps — status

The Phase-179/184 backend review's **blockers are closed**:

| Prior gap | Status |
|---|---|
| No default sandbox | ✅ Closed (Ch. H.180 — `[sandbox] default_backend = "auto"`) |
| Distribution dormant | ✅ Closed (Ch. Q + v0.2.0 cargo-dist binaries) |
| Thin first-launch identity | ✅ Closed (H.181 + W/X LLM-assisted persona seed) |
| No credential UX | ✅ Closed (`aivyx connect`, H.182) |
| Skills code-shaped | ✅ Closed (conversational skill-teaching, H.184) |

## 3. Findings — issues to resolve (ranked)

### 🟢 F1 — Cross-Site WebSocket Hijacking on `/ws` — **RESOLVED (this audit)**

The Studio `/ws` upgrade accepted any handshake regardless of `Origin`.
A loopback bind is not a boundary against the browser (WebSockets are
exempt from the same-origin policy), so a malicious page the operator
visited could open `ws://127.0.0.1:7843/ws` and drive the already-unlocked
daemon — materially worse since the Studio now **writes config** (access
level, budgets) and the **filesystem** (Documents editor).
**Fixed** by an `Origin` check on the upgrade (`web_ui.rs`,
`ws_origin_allowed`): accept only an absent `Origin` (non-browser client,
already inside the trust boundary via the Unix socket) or an exact loopback
origin on the bound port; cross-site / DNS-rebind / wrong-port / `null`
origins get a `403`. Verified at the wire level + unit-tested; documented
as [`THREAT_MODEL.md`](THREAT_MODEL.md) §4.11.

### 🟠 F2 — No tool-level rate limiting / quota

Capabilities gate *what* a tool may do, never *how often*.
`notify_dispatcher` has per-channel rate limits, but there is no general
per-tool / per-turn quota — relevant for a security-focused framework
running autonomous loops (a misbehaving loop can hammer
`web.fetch` / `shell.exec`). **Open.** Roadmap candidate: a capability-set
or turn-loop-level call budget, audited like other limits.

### 🟠 F3 — Channel Activation Milestone (cross-channel continuity)

Still open ([`ROADMAP.md`](ROADMAP.md) §"Channel Activation Milestone";
deferral markers in `aivyx-slack` / `aivyx-discord`). The largest *feature*
gap — the agent does not yet feel like *one* assistant across Local / voice
/ Discord / Slack / Telegram. Roadmap-sized, not a defect.

### 🟠 F4 — Everyday-PA domain breadth

Reminders (Phase 183) and budget/cost (Ch. K) landed, but contacts/CRM,
weather, maps/location, SMS/messaging, smart-home, and media remain absent.
The covered set still skews developer / knowledge-worker
(Gmail / Calendar / Drive / Notion / n8n).

### 🔵 F5 — Three unmaintained transitive dependencies

`fxhash` (RUSTSEC-2025-0057), `number_prefix` (RUSTSEC-2025-0119),
`paste` (RUSTSEC-2024-0436) — **unmaintained, not vulnerable**; zero CVEs.
Track for replacement when their parents update. (The repo's Dependabot
alerts were not readable via the current PAT — confirm on the Security tab;
these advisories are the likely content.)

### 🔵 F6 — `SystemTime`-seeded backoff jitter

`aivyx-voice/session.rs` `backoff_with_jitter`. Non-security
(thundering-herd timing), defaults to off. Key material is unaffected —
the store salt is `Uuid::new_v4()` (122 bits of `getrandom` CSPRNG) and
AEAD nonces likewise. Close only if you want a clean "no `SystemTime`
randomness anywhere" story.

### 🔵 F7 — Doc/code count drift

The scope-base count drifts between docs (83) and code (~88 `KNOWN_BASES`
literals). A one-line `assert_eq!(KNOWN_BASES.len(), N)` test — plus
equivalents for storage-domain and substrate-tool counts — would stop docs
diverging from code.

### Note (not a finding)

`aivyx-channel` carries 134 production `unwrap`/`expect`; sampled ones are
mutex-poison `expect`s (acceptable). A sampling pass to confirm none are
reachable on bad input would be worthwhile but is low priority.

## 4. What is *not* missing

Crypto (Argon2id → HKDF-SHA256 → ChaCha20-Poly1305, CSPRNG salts/nonces via
`getrandom`, zeroize-on-drop), the HMAC audit chain (offline-verifiable,
`ChainBroken` on tamper), capability enforcement in the turn loop, encrypted
storage (20 HKDF-isolated domains), and OAuth per-process token isolation are
all present, wired, and well-tested. The five founding pillars
(Local-First · Security-Focused · Customizable · Autonomous · general-purpose)
are intact.

## 5. Bottom line

Nothing architectural is missing. The one genuine *security* defect (F1) is
**fixed**. The remaining open items are roadmap-shaped (F2 rate-limiting/quota,
F3 cross-channel continuity, F4 PA-domain breadth) or cosmetic (F5–F7) — the
expected "broaden + harden" layer on a mature, disciplined substrate. These
are the natural seeds for the next roadmap:

1. **Harden:** tool-level rate limiting / quota (F2); count-assertion tests (F7).
2. **Unify:** the Channel Activation Milestone — one assistant everywhere (F3).
3. **Broaden:** everyday-PA tool domains (F4).

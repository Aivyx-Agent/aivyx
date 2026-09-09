# Aivyx PA Backend Framework — Audit Review

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
| No credential UX | ✅ Closed (`aivyx-pa connect`, H.182) |
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

### 🟢 F2 — No tool-level rate limiting / quota — **RESOLVED (Chapter Throttle)**

Capabilities gated *what* a tool may do, never *how often* — within a turn /
loop iteration / Nonagon mission, tool calls were unbounded. **Fixed** by
**Chapter Throttle**: a third dispatch gate (`RateGate`) checked after the
capability + role gates and before execute, bounding calls by the operator's
`[rate_limit]` caps (per-turn-per-tool, per-turn-total, sliding-window) with an
`alert` / `deny` action. A throttled call yields a forensically-distinct
`ToolOutcome::RateLimited` + a dedicated `RateLimited` audit record. Opt-in
(uncapped default), on the one chain. See [`RATE_LIMITS.md`](RATE_LIMITS.md).

### 🟠 F3 — Channel Activation Milestone (cross-channel continuity)

Still open ([`ROADMAP.md`](ROADMAP.md) §"Channel Activation Milestone";
deferral markers in `aivyx-slack` / `aivyx-discord`). The largest *feature*
gap — the agent does not yet feel like *one* assistant across Local / voice
/ Discord / Slack / Telegram. Roadmap-sized, not a defect.

### 🟠 F4 — Everyday-PA domain breadth — **first slice landed (Chapter Contacts)**

Reminders (Phase 183) and budget/cost (Ch. K) landed; **Chapter Contacts**
(the Broaden track's first chapter) adds the **contacts/CRM** primitive via the
`aivyx-contacts` People API tool process (six tools — see
[`CONTACTS.md`](CONTACTS.md)). Still absent: weather, maps/location,
SMS/messaging, smart-home, and media — the remaining Broaden slices. The
covered set still skews developer / knowledge-worker
(Gmail / Calendar / Drive / Notion / n8n + Contacts).

### 🟢 F5 — Three unmaintained transitive dependencies — **RESOLVED (deny.toml)**

`fxhash` (RUSTSEC-2025-0057), `number_prefix` (RUSTSEC-2025-0119),
`paste` (RUSTSEC-2024-0436) — **unmaintained, not vulnerable**; zero CVEs.
None is a direct Aivyx PA dependency: `fxhash` / `number_prefix` only enter the
build under the opt-in `mistralrs` local-LLM provider features (via
`bm25` / `indicatif`→`hf-hub`), and `paste` rides `frankenstein`
(`aivyx-telegram`). No code change removes them until those upstreams update.
**Resolved** by recording the decision in a root [`deny.toml`](../deny.toml):
the three advisories are ignored with parent-chain rationale, and
`cargo deny check advisories` (graph built `all-features = true`, so the
optional provider stacks are actually evaluated) is **green** while still
surfacing any *new* advisory. Revisit `paste` at the `frankenstein` 0.50 bump.

### 🟢 F6 — `SystemTime`-seeded backoff jitter — **RESOLVED**

`aivyx-voice/session.rs` `backoff_with_jitter` seeded its ±jitter offset
from `SystemTime` subsec-nanos. Non-security (thundering-herd timing),
defaults to off — but **fixed** for the clean "no `SystemTime` randomness
anywhere" story: the jitter now draws from the low 64 bits of
`Uuid::new_v4()`, the same `getrandom`-backed CSPRNG used for the store
salt and AEAD nonces. (Key material was never affected.)

### 🟢 F7 — Doc/code count drift — **RESOLVED (Chapter Throttle TH.4)**

The originally-flagged scope-base drift was a false alarm: `KNOWN_BASES.len()`
is already asserted `== 83` in code (the audit's "~88" was an over-counting
grep that included doc-comment examples). The *real* drift was the **storage
domain count** — code has **21** (`KeyDomain::ALL`) while the docs said 20 (the
Phase-183 `Reminders` domain was never propagated). **Fixed**: corrected the
docs to 21 and added a `key_domain_count_matches_docs` assertion with a
doc-update reminder, so the figure can no longer diverge silently.

### Note (not a finding)

`aivyx-channel` carries 134 production `unwrap`/`expect`; sampled ones are
mutex-poison `expect`s (acceptable). A sampling pass to confirm none are
reachable on bad input would be worthwhile but is low priority.

## 4. What is *not* missing

Crypto (Argon2id → HKDF-SHA256 → ChaCha20-Poly1305, CSPRNG salts/nonces via
`getrandom`, zeroize-on-drop), the HMAC audit chain (offline-verifiable,
`ChainBroken` on tamper), capability enforcement in the turn loop, encrypted
storage (21 HKDF-isolated domains), and OAuth per-process token isolation are
all present, wired, and well-tested. The five founding pillars
(Local-First · Security-Focused · Customizable · Autonomous · general-purpose)
are intact.

## 5. Bottom line

Nothing architectural is missing. The genuine *security* defect (F1) is
**fixed**, and the **Harden** track is now **closed** — F2 (tool-call rate
limiting / quota) shipped as **Chapter Throttle**, and F7 (count drift) is
resolved with a corrected storage-domain count + a drift-guard test. The
cosmetic findings are also closed — **F6** (SystemTime jitter → CSPRNG) and
**F5** (unmaintained transitive deps recorded in `deny.toml`, gate green).
The remaining open items are the two roadmap-shaped tracks (F3 cross-channel
continuity, F4 PA-domain breadth) — the expected "broaden/unify" layer on a
mature, disciplined substrate. The natural seeds for the next roadmap:

1. ~~**Harden:** tool-level rate limiting / quota (F2); count-assertion tests (F7).~~ ✅ done (Chapter Throttle). Cosmetic F5/F6 also closed.
2. **Unify:** the Channel Activation Milestone — one assistant everywhere (F3).
3. **Broaden:** everyday-PA tool domains (F4). First slice (Contacts) ✅ shipped.

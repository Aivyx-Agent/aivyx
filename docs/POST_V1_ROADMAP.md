# Post-v1.0.0 Ideas

A parking lot for things surfaced in conversation that are **explicitly
not being built now** — distinct from [`ROADMAP.md`](ROADMAP.md) (active
technical phases) and [`PRODUCT_ROADMAP.md`](PRODUCT_ROADMAP.md) (forward
product-shape milestones already committed to). Everything here is a
*maybe*, parked past v1.0.0, with a stated reason it isn't now and a
trigger condition for revisiting.

**This is not a roadmap in the scheduling sense — nothing here has a
phase number or a target version.** An idea graduates out of this file
by moving into `ROADMAP.md`/`PRODUCT_ROADMAP.md` once its trigger fires
and it's actually scheduled; until then it just sits here so it isn't
re-litigated from scratch every time it comes up again.

## How to add an entry

One entry per idea: what it is, why it came up, the tradeoff as best
understood today, and — most importantly — **the trigger**: the
condition that would make it worth pulling forward. No task lists, no
design work — if an idea needs that, it has outgrown this file and
belongs in a real roadmap entry or its own design doc.

---

## Enterprise / "Factory" layer (multi-instance orchestration)

**What:** a commercial control-plane layer — transport between daemon
instances, multi-tenant + enterprise SSO, a fleet-management dashboard,
centralized audit rollup — sitting on top of the SAME core, never a
fork of it.

**Why it came up:** operator asked whether the codebase should be
branched for a business/enterprise paid track.

**Status:** already designed, not parked here by accident — this is
the fullest-specified idea in this file. Licensing is already live
(BUSL-1.1 since v0.3.0: free personal, paid commercial — see
`COMMERCIAL.md`). The trust *substrate* Factory would sit on (identity,
cross-operator attenuation, consent gating) already shipped as Chapter
Passport (`docs/FEDERATION.md`), proven in-process. What's missing is
everything network-shaped: transport choice, multi-tenancy, the fleet
UI, and centralized audit aggregation.

**Trigger:** real multi-node demand — an actual business/team wanting
to run Aivyx PA across more than one person. Locked decision (2026-06-26):
never build this speculatively; it's a license-boundary layer, not a
second engine.

**See also:** the private `STRATEGY.md` (commercial layer detail, not
in this repo); `docs/FEDERATION.md` §9 phase plan, §10 open questions
(discovery, transport tech, key rotation — all still genuinely
undecided).

---

## vLLM provider support

**What:** add `ProviderKind::VLlm` alongside the existing
`OpenAi`/`Ollama`/`LlamaCpp`/`Jan` providers.

**Why it came up:** operator asked whether vLLM was worth adding as a
local-model backend.

**Status:** likely cheap. vLLM serves an OpenAI-compatible API, and
three existing providers already ride the same `aivyx-llm::openai`
code path via a `base_url` override — an operator can likely point
`[openai] base_url` at a running vLLM server *today* with zero new
code. A named provider variant would mainly buy a friendlier default
port, matching the `LlamaCpp`/`Jan` precedent.

**Why not now:** vLLM's value proposition (multi-GPU throughput,
serving many concurrent requests off shared hardware) is an
enterprise/multi-tenant signal, not a personal-agent one — closer to
the Factory layer above than to the local-first core. Also unverified:
whether vLLM's guided/structured-output wire format matches what
Chapter Emboss already sends for grammar-constrained tool-calling
(json_schema shape) — untested, not confirmed compatible.

**Trigger:** a real deployment need (someone running Aivyx PA for a team
off shared GPU hardware), or the Factory layer maturing to the point
multi-instance serving is in scope anyway.

---

## Lemonade SDK provider support (AMD Strix Halo / Ryzen AI NPU)

**What:** add `ProviderKind::Lemonade` — AMD's open-source local AI
server (github.com/lemonade-sdk/lemonade). Auto-detects hardware and
exposes an OpenAI-compatible endpoint, unlocking the NPU on Ryzen AI
300/400-series laptops (50 TOPS otherwise idle) plus Radeon GPU and
Strix Halo. Also serves image-gen and speech, not just text.

**Why it came up:** operator asked about it after the vLLM
conversation, then specifically flagged Strix Halo's growing market
traction as a reason to consider doing it sooner.

**Status:** same cheap-add shape as vLLM (OpenAI-compatible endpoint,
rides the existing code path) — and arguably a *better* fit for
Aivyx PA's personal-agent audience than vLLM, since it's about unlocking
hardware a home user already owns rather than enterprise throughput.
Distinct from the existing `aivyx-llm::mistral_rs` in-process provider,
which solves "zero-dependency embedded inference," not "exploit this
specific NPU."

**Why not now (decided 2026-07-07):** three reasons, market timing
aside — (1) it's off the locked phasing (v0.9 = UI polish, v1.0 =
verticals + web presence; a new LLM provider is neither); (2) no test
hardware exists on this project — nobody owns a Strix Halo box or
Ryzen AI NPU, so shipping it now would be exactly the un-dogfooded
trap Chapter Passport flagged for itself; (3) the same open
tool-calling wire-format question as vLLM is unverified for Lemonade
too. The add is mechanically cheap, which is precisely why waiting
costs nothing — this isn't a foundational decision that gets harder to
retrofit later.

**Trigger:** either real Strix Halo/Ryzen AI NPU test hardware becomes
available to this project, or an operator/customer with that hardware
actually asks for it.

---

## Persistent interactive terminal sessions (Chapter Tether)

**What:** a real interactive shell session the agent can keep talking
to across multiple turns — open it, send input, read accumulated
output, close it — distinct from the existing `shell.exec`, which is
deliberately one-shot (spawn, run, capture, die; no persistent cwd,
no surviving background process, no warm REPL).

**Why it came up:** operator asked "should the agent have their own
terminal?", then confirmed they meant a real interactive session, not
just a Studio view watching `shell.exec` calls happen live.

**Status:** scoped, not started — see the full design doc,
[`docs/TETHER.md`](TETHER.md), written 2026-07-07. Not a quick
addition: needs a new daemon-owned pty-session registry (mirroring
`TeamMissionService`/`LoopDriver`), a small tool family
(`terminal.open/send/read/close/list`), and its own Trusted-tier
capability base (P10 amendment-gated, the same process `git.read`/
`git.write` and `team.run` went through). The lifecycle question —
does a session die at turn-end, session-end, or survive a daemon
restart? — gates almost everything else and isn't resolved.

**Why not now:** it's real, non-trivial design work touching the core
containment model, not a bolt-on. Ward/Portcullis's sensitive-path
guard is a one-shot pre-execution text scan; it doesn't extend cleanly
to a persistent shell where the agent can drop into an arbitrary REPL
mid-session (see `TETHER.md` §3 for why this is an honestly-named
residual risk, not a gap to paper over, and why it's qualitatively the
same risk `shell.exec` already carries today — just open-ended instead
of one call). Also off the locked v0.9/v1.0 phasing.

**Trigger:** a real dogfood workflow one-shot `shell.exec` genuinely
can't serve — e.g. the operator wants Jarvis to start and keep
monitoring a long-running dev server or watch-build across a
conversation — not speculative "this would be nice."

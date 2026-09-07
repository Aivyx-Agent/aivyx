# Phase 207 — `aivyx` client integration for `aivyx-broker` (multi-process GPU-slot coordination)

**Cross-repo GPU-slot scheduling — [SHIPPED] 2026-09-07.**

## Goal

Following a strategic question about building a custom LLM inference
engine (answered by recommending against it, but surfacing two real
opportunities — an embedded `mistral.rs` port for `aivyx-coder`, shipped
separately, and this phase), scope the second, larger opportunity: a
coordination layer so `aivyx` and a delegated `aivyx-coder` process can
share one GPU-backed `llama-server`'s finite KV-cache slots safely,
rather than each picking a slot via its own private, in-process tracker
with zero awareness of the other.

Grounded directly against real code before any design work began, to
separate confirmed fact from the originating audit's assumption: the
client-side race is real — both `aivyx`'s `KvSlotPool` and
`aivyx-coder`'s equivalent hand out the lowest free slot id first, with
no cross-process coordination, confirmed via direct reads of both
trackers and their real call sites. But the server-side consequence is
narrower than assumed: verified against llama.cpp's own
`server-context.cpp` that a same-slot collision is already deferred
safely by `llama-server` itself (`get_available_slot` returns the busy
slot, but `process_single_task` separately checks `is_processing()` and
queues rather than running concurrently) — not a data-corruption bug,
but silent head-of-line blocking and KV-cache-locality thrash. Informed
of this narrower severity, the operator chose to build the full
scheduling layer anyway rather than a narrower collision-avoidance
patch.

## What shipped

A new standalone repo, `aivyx-broker` — a loopback-only Axum daemon
(no auth, same trust model as `llama-server` itself) sitting fully in
the request path between both client apps and the real `llama-server`.
It exposes an OpenAI-compatible `POST /v1/chat/completions` (so
pointing a client at it is a `base_url`-equivalent config change) plus
an additive `aivyx_slot_hint` field (`{prefix_hash, preferred_slot}`),
and owns both cache-locality-aware slot admission and the full
`aivyx-kvcache` restore/warm/save lifecycle — a design correction made
during implementation planning once it became clear a client can't
restore its own slot first when it doesn't learn the slot number until
the broker's own admission call returns.

This phase is the `aivyx`-side half of the client integration (a
sibling task did the equivalent for `aivyx-coder`, in that repo):

- **`ProviderKind::Broker`**, a new `is_openai_compatible()` provider —
  same OpenAI-compatible wire protocol as `ProviderKind::LlamaCpp`, just
  pointed at the broker's `base_url` instead of `llama-server`'s. A new
  `[broker] base_url` config section, defaulting to
  `http://127.0.0.1:8899` (the broker's own documented default bind).
- Broker mode skips this process's own local `KvSlotPool::checkout()`
  and `aivyx-kvcache` restore/save calls entirely — traced end to end:
  `kv_cache_handles` (and therefore `with_kv_cache`) is never
  constructed for this provider, so `ensure_kv_slot_checked_out`'s own
  early-return guard already no-ops the whole local lifecycle. The
  client only ever sends the `prefix_hash` hint it already computes
  today via its own existing `compute_prefix_hash` — no duplicate hash
  implementation.
- Threaded through every place this process can construct an `Agent`
  against the shared broker-pointed backend — not just the main
  agent — a real gap caught during code review, not anticipated by the
  plan: team-mission specialists (`aivyx-team`) share the same backend
  but hadn't been wired, unlike this codebase's existing parity for
  `LlamaCpp`'s own kvcache wiring. A hint-less request from an unwired
  specialist would land on the broker and silently clear whatever
  prefix a hinted request had just established there — actively
  defeating the feature for team-mission workloads (many specialist
  sub-turns, each with its own stable system prompt — exactly the
  workload cache-locality routing helps most), not merely failing to
  benefit from it. Fixed by mirroring the existing `kv_cache_handles`
  plumbing chain through all five files it actually spans
  (`aivyx.rs`, `aivyx_modules/team.rs`, `aivyx-channel/team_mission_driver.rs`,
  `aivyx-team/assembly.rs`, `aivyx-team/factory.rs`) rather than stopping
  at the first one found.
- Two more real defects surfaced by the same review round: the
  `--provider broker` CLI flag rejected the very provider its own TOML
  config and env var already accepted (a third, independent parse site
  the earlier work missed); and the startup config banner printed
  `[openai] base_url` — a field broker mode doesn't read at all — for a
  provider that's `is_openai_compatible() == true` but has its own
  dedicated `[broker] base_url`, silently mis-stating which address
  requests actually go to on the exact `llamacpp → broker` migration
  path this feature exists to support.

## The result

An operator running `aivyx`'s daemon and a delegated `aivyx-coder`
subprocess against the same `llama-server` can now point both at
`aivyx-broker` instead, getting cache-locality-aware slot admission and
FIFO-ish queuing across the two processes rather than each blindly
picking slot 0 first. Purely additive — every existing provider's
behavior, request shape, and test coverage is unchanged; broker mode is
strictly opt-in via `provider = "broker"`.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx-broker` itself** (the daemon this phase's client integrates
  against) has its own follow-ups logged in its own repo — see
  `aivyx-broker/docs/superpowers/specs/2026-09-07-aivyx-broker-design.md`'s
  "Out of scope for v1" section (no priority/weighted scheduling, no
  remote/multi-host operation, no automatic lifecycle management).
- **`council.rs`/`architect.rs` construct their own, separately
  configured `OpenAiCompatBackend`s** from `[council].members[]`/
  `[architect]` `base_url`s, never the shared broker-pointed backend —
  confirmed out of scope, not a gap, since those features intentionally
  talk to their own independently-configured endpoints.
- **`--base-url` CLI override and the context-window probe are both
  effectively inert in broker mode** (the probe is now explicitly
  skipped; `--base-url` silently has no effect since broker mode reads
  `broker_base_url` instead) — both documented behavior, not bugs, but
  worth a clearer error/warning if a future pass wants to close the gap
  between "silently ignored" and "explicitly rejected."

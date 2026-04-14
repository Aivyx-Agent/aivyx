# Aivyx

Agent-first Rust framework for building AI agents with capability-based
security, auditable tool execution, and trust-tiered channel support.

See [DESIGN.md](DESIGN.md) for the locked design contract — the north
star for this rebuild. Per-phase working records and the amendment
process live under [`docs/`](docs/README.md).

## Status

**Phases 0–7 complete.** The design contract (D1–D8) has been locked
since commit `1b4f271` and held unchanged across **seven** consecutive
implementation phases. The working agent ships with:

- a capability-typed tool registry and turn loop (`aivyx-core`),
- a **persistent** HMAC-chained audit log (`aivyx-audit`) wired into
  every turn — the chain survives restarts, `aivyx --verify-only`
  cold-verifies the full history without needing `ANTHROPIC_API_KEY`,
  and a tampered audit row trips `AuditError::ChainBroken` on the
  next open,
- a streaming Anthropic provider with wall-clock cancellation
  (`aivyx-llm`),
- `fs.read` / `fs.write` as first concrete, scope-checked tools,
- an encrypted per-domain KV store over redb with Argon2id-derived
  keys and ChaCha20-Poly1305 AEAD (`aivyx-storage` + `aivyx-crypto`),
  with the store file `chmod 0600` on cold start,
- **`memory.read` / `memory.write` / `memory.forget` as D1-faithful
  memory tools** (`aivyx-memory`) — the agent recalls across
  restarts because it *chose to call* the tool, not because a prompt
  hook silently injected history, and the audit chain carries
  per-topic `memory.<op>:topic:<topic>` scopes to prove it,
- a per-topic memory size tripwire (`AIVYX_MEMORY_MAX_PER_TOPIC`,
  default 10 000) that refuses runaway writes with a typed `Failed`
  outcome the planner can observe,
- a CLI binary (`aivyx`) that composes all of the above, reads its
  passphrase from `AIVYX_PASSPHRASE` **or** prompts interactively via
  `rpassword` when stdin is a tty, persists session state and memory
  under `$XDG_DATA_HOME/aivyx/store.redb`, and exposes `--verify-only`
  as a read-only forensic entry point.

Phase 8 (Ecosystem — Telegram adapter) is next — see
[`docs/ROADMAP.md`](docs/ROADMAP.md) and
[`docs/PHASE_8.md`](docs/PHASE_8.md). For the frozen per-phase
records, see [`docs/`](docs/README.md).

## License

Code is MIT-licensed. See [LICENSE](LICENSE). The "Aivyx" name and
associated branding are trademarked — see [TRADEMARK.md](TRADEMARK.md)
for the brand usage rule.

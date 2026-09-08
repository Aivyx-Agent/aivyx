# Phase 208 — `aivyx` client integration for `aivyx-yubi` (hardware-backed federation identity)

**Cross-repo hardware-security integration — [SHIPPED] 2026-09-08.**

## Goal

A second security-focused idea, raised directly by the operator ("security
is becoming a major issue in the tech/AI space") and scoped independently
from the `aivyx-broker` GPU-slot-coordination work: could a YubiKey harden
any part of this ecosystem? Grounding surfaced three real, independent
candidates — hardware-backed federation identity, a physical
confirm-to-execute gate on dangerous agent actions, and a challenge-response
hardening of the storage master-key unlock (the last of which would touch
DESIGN.md's locked D7 storage contract and require a formal amendment). The
operator chose the first, judging it the most concrete, lowest-risk starting
point: `aivyx-federation`'s `Identity` (the operator-owned Ed25519 keypair
signing every cross-boundary federation request) is generated in software
today and sealed at rest with a key derived from the storage master key —
protected, but the private key material still exists (encrypted) on disk
and in process memory whenever the daemon runs. Moving it into a YubiKey
means the private key never exists outside the device at all, and every
signature requires a physical touch.

Real crate/hardware landscape was grounded directly before any design
work began, and it ruled out the first obvious-looking path: the `yubikey`
Rust crate (PIV applet) is unmaintained since August 2023 and lacks Ed25519
support even though firmware 5.7+ (2024) added it to PIV; `openpgp-card`
(OpenPGP applet) is actively maintained and has supported Ed25519 since
firmware 5.2.3 (2019) — the real path, not the more obvious-looking one.

## What shipped

A new standalone repo, **`aivyx-yubi`** (`github.com/Aivyx-Agent/aivyx-yubi`,
created and pushed this same effort — a real, outward-facing action taken
mid-implementation once it became clear `aivyx-federation` genuinely needs
it as a Cargo dependency, not just an HTTP-reachable service like
`aivyx-broker`) — a focused primitive crate wrapping a YubiKey's OpenPGP
card applet: card discovery, on-card Ed25519 key generation for the
Signature slot, fixed/always-on touch-policy enforcement, PIN handling, and
raw-byte signing. It has no knowledge of Aivyx's federation protocol at all;
`aivyx-federation`'s `Identity` is its first consumer.

This phase is the `aivyx`-side integration:

- **`Identity` gained an internal `IdentitySigner` enum** (`Software`/
  `Hardware`), and `sign_request`/`sign_relay` became `async fn` to
  accommodate a touch-required hardware signature blocking for several
  seconds — confirmed via direct grounding that `aivyx-federation` had
  **zero production call sites anywhere in the workspace** before this
  phase (every existing call was inside the crate's own tests), so this
  is the first time `aivyx-federation` has ever been wired into a real
  binary (`aivyx-cli`) at all.
- **`Identity::load_hardware(instance_id, signer, expected_serial)`** takes
  an already-constructed `aivyx_yubi::YubiKeySigner` rather than a raw PIN
  — keeping PIN-acquisition UX entirely out of `aivyx-federation`'s scope,
  deferred to the CLI that actually collects it.
- **A new `aivyx federation yubikey-init` CLI subcommand**
  (`crates/aivyx-cli/src/bin/aivyx_modules/federation.rs`) drives the full
  provisioning flow: validate the instance id → discover the card → refuse
  a factory-default PIN → generate the Signature-slot Ed25519 keypair →
  set touch-policy fixed → write a non-secret binding record
  (`{instance_id, card_serial, public_key_base64}`).
- **Both new dependencies (`aivyx-yubi` on `aivyx-federation`, and
  `aivyx-federation`/`aivyx-yubi` on `aivyx-cli`) are `optional`, gated
  behind a new `yubikey` Cargo feature, default-off** — `aivyx-yubi`
  transitively needs `libpcsclite` at build time (via `pcsc-sys`), which
  isn't installed on every contributor's machine or in the default CI job,
  so a plain `cargo build --workspace` must never be affected. CI now runs
  a dedicated `--features yubikey` step.

**Two critical, structurally-real findings, both independently
re-verified and fixed:**

- **A Cargo-level dependency-resolution bug, not a code bug**: the first
  attempt at feature-gating used `aivyx-yubi = { path = "../../../aivyx-yubi", optional = true }`
  — but Cargo resolves `path` dependencies at manifest-load time
  *regardless* of whether the feature gating them is enabled, meaning the
  sibling directory had to exist on disk even with `yubikey` off. Proven by
  simulating a real CI checkout (`git archive` into a fresh temp directory)
  — `cargo clippy --workspace` failed at manifest resolution on every run,
  before any hardware-specific step. The real fix required `aivyx-yubi` to
  get an actual GitHub remote and switch to an optional `git` dependency
  (pinned by commit SHA, centralized in the root `Cargo.toml`'s
  `[workspace.dependencies]`) — the same pattern `aivyx-confine`/
  `aivyx-checkpoint`/`aivyx-kvcache` already use. Re-verified with an
  isolated `CARGO_HOME` forcing a genuine fresh network fetch from the
  real remote, from a location outside this whole workspace, with no local
  cache or symlink.
- **A real PC/SC deadlock in the CLI's own provisioning flow**: the
  original implementation held an open exclusive PC/SC transaction through
  key generation and touch-policy setting, then — without releasing it —
  attempted a "verification pass" that opened a *second* exclusive
  transaction on the same physical reader. `SCardBeginTransaction` blocks
  indefinitely when another exclusive transaction is already held. On real
  hardware, this meant the command would hang forever *after* the card had
  already been irreversibly re-keyed — a real, safety-relevant bug caught
  in review, not in production. Fixed by explicitly dropping the first
  transaction and card handle before the second connection attempt;
  verified against the real vendored `openpgp-card`/`pcsc`/
  `card-backend-pcsc` source (confirming exactly where `SCardEndTransaction`/
  `SCardDisconnect` actually fire) and by empirically compiling a
  deliberately-reversed drop order to confirm the borrow checker itself
  enforces the correct sequence.

Also fixed in the same review passes: the CLI's own "verification pass"
initially compared a card's serial against itself (a tautology) while
never re-checking the one property that actually matters — whether the
touch-policy setting genuinely stuck — corrected to honest messaging about
what was and wasn't actually confirmed; instance-id validation originally
ran *after* the card had already been destructively re-keyed rather than
before any card I/O; no warning existed that a failed Admin-PIN attempt
consumes real, non-recoverable retry budget (two mistakes, not three, can
permanently brick the Admin PIN with no self-recovery short of a full card
wipe) — now surfaced clearly in both the CLI's own error output and
`docs/INSTALL.md`.

## The result

An operator can now provision a YubiKey to hold `aivyx`'s federation
identity entirely in hardware — the private key never exists outside the
device, and every cross-boundary federation signature requires a physical
touch. Purely additive and opt-in: every existing software-generated
identity, and the default `cargo build`, are completely unaffected unless
`--features yubikey` is explicitly requested.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx-yubi` itself** has its own follow-ups logged in its own repo
  (`docs/superpowers/specs/2026-09-07-aivyx-yubi-design.md`'s "Out of
  scope" section) — no PIV support, no real-hardware-verified test suite
  (all tests run against a fake card backend; this environment has no
  real YubiKey), and the two other candidate use cases surfaced during
  scoping (a physical confirm-to-execute gate on dangerous agent actions,
  and challenge-response hardening of the storage master-key unlock) were
  explicitly deferred, not ruled out.
- **The CLI's own "verification pass" cannot re-confirm live touch
  enforcement** without a full sign attempt (which would need to collect
  and use the User PIN mid-provisioning, not this command's job) — the
  honest scope limitation is documented in the CLI's own output rather
  than papered over.

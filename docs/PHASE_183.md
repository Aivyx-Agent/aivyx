# Phase 183 — Reminders (Everyday-PA Domain Breadth #1)

**Chapter H, phase 4** — the first of the everyday-PA breadth
sub-sequence the roadmap flagged. The backend review found the
tool set skews developer / knowledge-worker; the operator picked
**reminders** as the first everyday capability to add. *("Remind
me to call mom at 6pm" → a notification arrives at 6pm.)*

## An honest packaging correction

The domain was chosen under the framing "a new tool in the
`aivyx-toolkit` bundle." On inspection that's the **wrong home**:
a real reminder must **push** a notification *at a time*, and the
toolkit is a separate, pull-only tool process (its `health.check`
polling surfaces state the agent *reads*; it has no way to notify
the operator at a moment). Pushing a message at time T is exactly
what the **daemon** already owns — the scheduler cadence + the
notify dispatcher. So reminders ship **daemon-native**, as
**channel-tier tools** (the established `mission.*` / `loop.*` /
`reflection.*` pattern), *not* in the toolkit. Same capability;
correct home. (The 13-tool substrate cap is untouched —
channel-tier tools are distinct from it, per DESIGN A12.)

## Design

A reminder is a durable *"at `due`, notify with `message`"* row,
fired by a background driver that pushes through the existing
notify dispatcher.

- **`KeyDomain::Reminders` + `ReminderStore`** — a simple durable
  CRUD store (not the HMAC-chained kind; reminders are
  operator-set, not security events): `set`, `list` (pending),
  `cancel`, and a pure **`due_now(reminders, now)`** selector.
  Each row: `id`, `due_unix`, `message`, optional
  `notify_targets`, `created_unix`.
- **`remind.*` channel-tier tools** —
  - `remind.set { at, message, notify_targets? }` — `at` is an
    **absolute** time (unix seconds or RFC3339). The **agent**
    resolves natural language ("6pm", "in 2 hours") to an
    absolute time using the turn's clock context, so no
    date-parsing dependency lands in the tool.
  - `remind.list` — pending reminders, soonest first.
  - `remind.cancel { id }` — drop a pending reminder.
  - New capability bases `remind.read` / `remind.write` (the
    capability-taxonomy-growth process — `KNOWN_BASES` + the
    count tripwire + the A3 amendment; **not** a `DESIGN.md`
    edit, exactly as the Phase 172/173 scope additions were).
- **The reminder driver** — a re-arming background task (sibling
  of `reflection_scheduler` / `loop_driver`): on each tick, find
  reminders whose `due_unix <= now`, **dispatch** the message to
  its targets (or the operator's default notify targets) via the
  notify dispatcher, and mark them fired (remove). A
  `[reminders].check_interval_secs` knob bounds the tick; absent
  → a sensible default.

## Why daemon-native is the right fit

- The notify dispatcher (`dispatch(message, targets)`) and the
  re-arming driver pattern are already daemon machinery.
- Channel-tier tools already CRUD daemon stores (`loop.*` over
  the backlog) and declare `KNOWN_BASES` scopes — `remind.*`
  slots straight in.
- A reminder is *push*; the toolkit is *pull*. The capability
  literally cannot live correctly in a separate process.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 182's frozen hash (`c600ab0`).
2. **`ReminderStore` + `KeyDomain::Reminders`.** The store
   (`set` / `list` / `cancel`) + the pure `due_now` selector +
   the storage-domain plumbing (enum, `as_bytes`, `table_name`,
   the ALL array + the count tripwire). Tests: CRUD round-trip,
   `due_now` boundary (past fires, future doesn't, exact-now
   fires), list-soonest-first.
3. **`remind.*` tools + scopes.** `remind.set` / `remind.list` /
   `remind.cancel` channel-tier tools + the `remind.read` /
   `remind.write` bases (`KNOWN_BASES` + tripwire + A3 amendment)
   + registration in the daemon tool list. Tests: each tool's
   behaviour + `required_scope` + absolute-time parsing
   (unix + RFC3339).
4. **The reminder driver + config.** The re-arming driver (due →
   dispatch → mark fired), `[reminders].check_interval_secs`
   config, and the bin wiring (spawn alongside the other
   drivers; thread the store + dispatcher). Tests: the
   fire-and-clear cycle with a fake dispatcher; default-targets
   fallback; the config knob.
5. **INSTALL + exit + Frozen.** INSTALL section (the `remind.*`
   tools, the agent-resolves-NL note, the notify-target
   defaults); exit doc; README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 19 → **20**. A new
  `KeyDomain` + channel-tier tools + capability bases — every one
  of which the Phase 172/173 self-learning + loop work added
  *without* a `DESIGN.md` edit. The 13-tool substrate cap is
  untouched; the scope growth rides the A3 amendment process, not
  the contract.
- **PRODUCT.md** — **Will hold.** Streak: 73 → **74**. An
  additive everyday-PA capability (like the already-shipped
  budget tools); not a new commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 19 →
  **20**. Work lands in `aivyx-channel` + `aivyx-storage` +
  `aivyx-capability` + `aivyx-config`; `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_183.md` + README row + Phase 182 backfill — T1.
- [ ] `ReminderStore` + `KeyDomain::Reminders` + `due_now` — T2.
- [ ] `remind.set/list/cancel` + `remind.read/write` bases — T3.
- [ ] The driver fires due reminders via notify + clears them — T4.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+18` to `+28`. *(A substrate phase like
  the loop foundation: a durable store + a pure selector + three
  tools + a driver fire-cycle. Denser than the recent
  glue/UX phases.)*

## Honest scope risks at sign-off

- **The agent resolves natural-language time, not the tool.**
  `remind.set` takes an absolute time; "6pm" → a timestamp is the
  LLM's job (with the turn clock in context). A model that
  miscomputes the time sets the wrong reminder — the
  `remind.list` readback is the operator's check.
- **Fired delivery is best-effort.** A reminder due while the
  daemon is down fires on the next start *if still future-or-now*;
  a missed-while-down reminder fires late (at next tick) or is
  dropped if long past — documented, with a grace policy.
- **Notify targets must exist.** A reminder with no target + no
  configured default has nowhere to go; surfaced at `remind.set`
  time, not silently.
- **No recurrence in v1** — one-shot only. Recurring reminders
  ("every weekday at 9am") are the existing `[[schedule]]` cron
  surface; a `remind.*` recurrence sugar is a possible follow-on.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 20 | Untouched (a `KeyDomain` + channel-tier tools + bases, all added without a DESIGN edit in 172/173; A3 amendment for the scopes) | ✅ |
| PRODUCT.md HOLD → 74 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 20 | Untouched | ✅ |
| Zero new workspace deps | reused `chrono` (already present), notify, storage | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+18` to `+28` | **`+14`** (store 5 + tools 5 + driver 3 + config 1); ~4,181 → ~4,195 | ❌ **below band** |

**Band note — I regressed to a coarse label, and missed.** I
priced this `+18..+28` by calling it "a substrate phase like the
loop foundation." That was the wrong instinct — the *exact*
mistake the Phase 179 retro named. This substrate is **much
lighter** than the loop foundation: a plain CRUD store (not an
HMAC chain with a `decide()` function and gate verification),
three *thin* CRUD tools, and a simple fire-loop. Pricing by the
actual dense components — a simple store (~5) + 3 thin tools (~5)
+ a simple driver (~3) + a config knob (~1) — gives **~14**,
exactly what landed. The lesson, restated for the fifth time and
clearly not yet a reflex: **price by the components a phase
contains, never by a family label** ("substrate" / "loop" /
"glue"). Two substrate phases can differ 2×.

What shipped, end-to-end:

1. **`KeyDomain::Reminders` + `ReminderStore`** (T2). A simple
   id-keyed durable CRUD store + the pure `due_now` selector.
2. **`remind.*` tools + bases** (T3). Three channel-tier tools;
   `remind.read` / `remind.write` via the A3 process; the agent
   resolves NL time to an absolute `at`.
3. **The driver + config** (T4). A re-arming driver that delivers
   due reminders through the notify dispatcher (at-least-once)
   and clears them; `[reminders].check_interval_secs`.

### Honest scope risks at sign-off

- **The agent resolves NL time, not the tool** — `remind.list` is
  the readback check.
- **Delivery is best-effort / at-least-once** — a reminder due
  while the daemon is down fires on the next tick after restart;
  a failed clear re-fires (never lost).
- **No-target reminders fan out to every configured target** —
  the documented default.
- **One-shot only** — recurrence stays the `[[schedule]]` surface.

### The result

The first everyday-PA breadth pick: reminders, the canonical
personal-assistant capability, shipped daemon-native (the correct
home for push-at-a-time) with the 13-tool substrate cap untouched.
Streaks held; zero new deps. THREAT_MODEL updated to 20 domains.

# Team-Command Sender Allowlist Design

**Status:** Shipped 2026-08-24 — plan at
`docs/superpowers/plans/2026-08-23-team-command-sender-allowlist.md`,
merged to `main` at `0bd05511`. Closes the sender-restriction gap Piece
B's own final review deferred (see below).

The final whole-branch review found a real, security-relevant gap this
design doc did not anticipate: `/team run`'s own confirm-first flow (a
bare "yes"/"no" reply, not a `/team`-prefixed command) doesn't parse as
a `/team ...` command at all, so it skipped the allowlist check entirely
— any sender in the chat, not just the one who staged the trigger, could
confirm or cancel another sender's pending mission-start within the
5-minute window. Fixed by making `PendingTrigger` generic over the
sender-id type and recording who staged each trigger, so only that same
sender's confirm reply resolves it — a mismatched sender's reply falls
through silently (no denial reply, to avoid revealing a pending
trigger's existence to an uninvolved chat member) and the original
trigger survives untouched, still waiting for its real sender. The
review's own re-review independently re-derived the fix's mutation-proof
on a different channel than the one the fix report demonstrated before
approving.

The review also found this initiative's now-fourth consecutive instance
of the same pattern: a new, deny-by-default, deliberately
upgrade-breaking config knob shipped with zero operator documentation —
fixed before merge, correcting one doc (`docs/INSTALL.md`) that had
actively claimed something no longer true.

## Motivation

The Team-Mission Triggers initiative (Pieces A, B, and C, all shipped
2026-08-23) added a `/team ...` chat command surface to Telegram,
Discord, and Slack: `status [<id>]`, `approve|reject <id> <step>`,
`pause|resume <id>`, `abort <id>` (Piece B), and `run <goal>` (Piece C).

Piece B's own final whole-branch review found a real, deliberately
deferred gap: **neither the Discord nor Slack daemon-frontend has any
sender-level restriction on these commands at all.** Telegram has an
optional `chat_filter` (which *chat* the bot listens to), but nothing on
any of the three platforms restricts *who, within an already-connected
chat*, may issue a `/team` command. Anyone who is a member of an invited
Discord/Slack channel — or anyone in a Telegram chat the bot is
configured to serve — can `/team abort` a running autonomous multi-agent
mission or `/team approve`/`/team reject` a human-approval gate meant for
a specific operator, with zero opt-in required. This mirrors a
pre-existing, already-accepted trust model for the *old*, single-agent
mission gate system (`gate_command.rs`'s own `/approve`/`/reject`,
documented in `docs/THREAT_MODEL.md`) — so it is not a new regression —
but Team-Mission Triggers widens the blast radius from single-agent
mission gates to full autonomous multi-agent mission control, including
unilateral abort of a running mission.

Piece C's own `/team run` already has a channel-level gate
(`team_run_channel`, a per-channel-type operator opt-in the daemon
enforces server-side) — but that answers "which *channels* may start
missions at all," not "which *people*, within an opted-in channel, are
trusted to." The gap this design closes is specifically the sender
dimension, which currently doesn't exist for any `/team` command on any
of the three channels.

The user explicitly declined to fold a fix into Piece B or C, choosing
to scope it as its own follow-on plan — this design.

## What already exists (verified against real code)

- All three platforms' `IncomingMessage` structs already carry a
  platform-authenticated sender id on every message: Telegram's
  `user_id: i64`, Discord's `author_id: u64`, Slack's `user_id: String`
  (`crates/aivyx-telegram/src/transport.rs:67`,
  `crates/aivyx-discord/src/transport.rs:70`,
  `crates/aivyx-slack/src/transport.rs:70`). These are populated from
  the underlying platform API's own authenticated response, not from
  anything the connecting process asserts unverified — closing this gap
  requires no new plumbing to obtain sender identity.
- `TelegramConfig`/`DiscordConfig`/`SlackConfig`
  (`crates/aivyx-config/src/lib.rs`) already gained precedent for a new,
  simple, `#[serde(default)]` per-channel-type config knob via Piece C's
  own `team_run_channel: bool` / `team_trigger_rate_limit: Option<u32>` —
  the exact convention (raw TOML struct + public `Sourced`-free struct +
  conversion-site wiring) this design's own new field follows.
- `gate_command.rs`'s `/approve`/`/reject` parser is a **separate**
  system (the old, single-agent mission gate), explicitly out of scope —
  the trust model it operates under is pre-existing and already accepted
  in `docs/THREAT_MODEL.md`; this design touches only the `/team ...`
  surface (`team_command.rs`'s own parser domain).
- Piece C's own `handle_telegram_incoming_command` /
  `handle_discord_incoming_command` / `handle_slack_incoming_command`
  functions (added specifically to make the `/team run` vs. generic
  `team_command::parse` dispatch *ordering* directly testable, after a
  real bug in that ordering shipped and needed two review rounds to
  genuinely fix) are the natural home for this new check — they already
  own the full `/team` command precedence chain per channel.

## Architecture

**Dimension:** a per-sender allowlist, not a per-chat one. Telegram's
existing `chat_filter` (which chat the bot listens to at all) is a
different, complementary control this design doesn't touch or
generalize — the actual gap is about which *people*, once in an
already-connected chat, may act.

**Scope:** one allowlist per channel *type* (all Telegram, all Discord,
all Slack — not per specific chat instance), since a sender id is a
property of a specific human, not scoped to a particular chat. Gates the
**entire `/team` surface uniformly** — `status`/`approve`/`reject`/
`pause`/`resume`/`abort`/`run` all require allowlist membership. One
gate, one mental model; mission status is itself operationally sensitive
information, not something to leave open by default.

**`/team run` gets both its existing gate and this new one.**
`team_run_channel` (channel-level: is this chat opted into
mission-starting at all) and the new sender allowlist (person-level: is
this specific sender trusted) are orthogonal and both apply — today, any
sender in a `team_run_channel = true` chat can trigger a new mission;
this design closes that residual gap too.

**Default: empty/absent allowlist = deny all senders.** Consistent with
this initiative's own established deny-by-default posture (`team_run_channel`'s
own `false` default) — the fix closes the gap immediately on upgrade
rather than shipping inert until an operator discovers and configures
it. This is a real, deliberate behavior change for any deployment
currently relying on today's open access; the fix is only meaningful if
it's on by default.

**Enforcement location: client-side, in each channel-adapter process —
not daemon-side.** This differs from Piece C's own `team_run_channel`
architecture decision (which chose real server-side enforcement over a
client-side-only check) for a concrete reason verified against real
code: the sender id being checked here is authenticated by the
platform's own API before the channel-adapter process ever constructs an
`IncomingMessage` — it is not something the client asserts unverified,
unlike the `FrontendType` identity problem Piece C solved. Additionally,
Piece B's own commands (`status`/`approve`/`reject`/`pause`/`resume`/
`abort`) reach the daemon over the anonymous one-shot `Query` IPC path
with **no session identity at all** — the same structural gap Piece C
had to build new IPC specifically to work around for `/team run`.
Enforcing this allowlist server-side for Piece B's commands would
require redesigning their entire IPC shape (the identity-declaring
`StartSession`-based pattern, extended to also carry sender identity) —
a much larger, cross-cutting change for a security benefit that's
marginal given the channel-adapter is already fully trusted,
operator-deployed software (the same trust level as its own config
file). Client-side enforcement, checking a platform-authenticated value,
in trusted software, is the right-sized fix.

## Data flow

New fields on `TelegramConfig`/`DiscordConfig`/`SlackConfig`
(`crates/aivyx-config/src/lib.rs`), mirroring `team_run_channel`'s own
`#[serde(default)]` convention exactly — native-typed per platform's own
real id type, not a stringly-typed compromise:

```toml
[telegram]
team_command_allowed_senders = [123456789, 987654321]   # Vec<i64>

[discord]
team_command_allowed_senders = [111111111, 222222222]   # Vec<u64>

[slack]
team_command_allowed_senders = ["U012ABCDEF", "U098ZYXWVU"]  # Vec<String>
```

**Enforcement point:** the very top of each channel's own
`handle_*_incoming_command` function — before `handle_*_team_run_message`
or the generic `team_command::parse` dispatch ever run. If the inbound
text parses as *any* `TeamCommand` variant and the message's sender id
is not present in the configured allowlist, return the denial reply
immediately. Non-`/team` text (including `/cancel` and `/approve`/
`/reject`, which belong to `gate_command.rs`'s separate system) is
completely unaffected — it falls through to its existing handling
exactly as today, regardless of sender.

## Error handling

- Unauthorized sender issuing any `/team ...` command → a clear,
  `✗`-prefixed denial reply, never silence, matching every other denial
  pattern this initiative has established (`team_run_channel`'s own
  denial, the rate-limit denial, the capability-denied reply).
- No change to any existing non-`/team` message handling.

## Testing

- Pure config load/round-trip tests for the three new fields, mirroring
  `team_run_channel`'s own existing test shape.
- Pure allowlist-membership-check tests (sender in list / not in list /
  empty list denies / list absent denies).
- **An ordering-invariant test that would genuinely fail if the check
  were removed or moved after the command-dispatch logic it's meant to
  gate** — not a test of an extracted helper's own internals in
  isolation. Piece C's own final review found, via actual mutation
  testing, that its first attempt at an analogous ordering-invariant
  test only satisfied a finding's letter, not its defect (the test
  exercised an extracted function's internals, never the real call-site
  order). This design's own implementation plan must get this right on
  the first attempt: the test must exercise the *whole*
  `handle_*_incoming_command` function, asserting an unauthorized sender
  issuing `/team status` gets the denial reply and never reaches
  `team_dispatch::dispatch` at all.

## Out of scope

- `gate_command.rs`'s `/approve`/`/reject` parser (the old single-agent
  mission gate system) — a separate, pre-existing, already-accepted
  trust model, not touched by this design.
- Per-chat/channel-instance allowlist granularity (e.g. different
  trusted senders for different Slack channels the bot is in) — YAGNI;
  one allowlist per channel type is the minimal correct shape for the
  stated problem.
- Server-side (daemon) enforcement — see Architecture's reasoning above;
  would require a disproportionate redesign of Piece B's own IPC shape.
- Any UI/tooling for operators to *discover* Telegram/Discord/Slack user
  ids to populate the allowlist with — out of scope; operators already
  need platform-specific tooling to find these ids today for other
  purposes (e.g. Telegram's `chat_filter` already requires knowing a
  numeric chat id).

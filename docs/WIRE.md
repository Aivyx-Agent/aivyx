# Turns Down a Pipe — headless multi-turn sessions (Chapter Wire)

> **Status: SCOPED 2026-07-04.** `aivyx --headless "<task>"` is
> one-task-per-process: every invocation is a fresh session, so no
> same-session conversation can be driven from a terminal, a script, or a
> test harness. The Chapter Strop live verification hit this wall
> directly — a correction (a turn the operator immediately reworks) is
> *definitionally* a same-session pair, so the correction retro-fold
> could not be live-staged from the CLI. Wire adds the missing mode:
> pipe newline-delimited turns into `aivyx --headless` and they run as
> consecutive turns of **one** daemon-routed session. No new tool,
> capability base, amendment, IPC message, or dependency — the daemon
> already supports exactly this (a `DaemonSession` is stable across
> `submit_input_headless` calls); only the CLI never offered it.

## 1. Why — the session is the unit the CLI can't reach

Everything session-scoped is invisible to headless callers today:
conversation continuity, session-partitioned memory, and every
consecutive-turn signal (the correction proxy `followup_outcome`, and
through it Strop's retro-fold). Operators scripting the agent get the
same ceiling: a batch file of related steps runs as unrelated one-shot
strangers. The daemon-side machinery is complete — Chapter H's headless
posture (gates refuse instead of parking) applies per-submit, and the
interactive REPL already drives multi-turn sessions over the same IPC.

## 2. Architecture & decisions (locked)

- **Invocation shape:** `aivyx --headless` with **no task argument** and
  **non-TTY stdin** reads stdin line by line; each non-empty line is one
  turn in a single `DaemonSession`. EOF ends the session.
  `aivyx --headless "<task>"` is byte-identical to today. Bare
  `--headless` on a TTY keeps erroring, now with the pipe hint.
- **Fail-fast:** the stream stops at the first non-completed turn and
  exits with that turn's existing Chapter-H code (`3` gate-refusal, `1`
  other) — later lines in a broken conversation are nonsense, and batch
  callers keep the branchable codes. All turns completed → `0`.
- **Rendering unchanged:** each turn streams via the shared
  `render_for_cli` to stdout and its `aivyx --headless: <outcome>` line
  to stderr, exactly like the one-shot path — a pipe consumer sees the
  same shape N times.
- **Headless posture per turn:** every line submits with
  `headless: true`; nothing about Chapter H's gate semantics changes.
- **Blank lines are skipped**, not empty turns.

## 3. Scope

**In:** the stdin session loop; the parser change (bare `--headless`
becomes the stdin mode instead of a usage error when piped); unit tests
for the pure pieces; live rig verification — which doubles as the
outstanding **Strop retro-fold live proof** (two piped turns: a skill
turn, then an immediate same-session follow-up = a correction by the
shared `followup_outcome` proxy → the `retro-folded` breadcrumb).

**Out:** an interactive multi-turn TTY mode (that's the REPL);
per-line attachments/flags; a session-resume flag (`--session <id>`)
across processes — a natural follow-on if scripting demands it, but it
adds an IPC surface Wire deliberately doesn't need.

## 4. Phase plan

| Phase | What | Proof |
|---|---|---|
| **WI.0** | This doc. | Reviewed. |
| **WI.1** | `run_headless_stdin` in the headless module (one connect, loop over lines, fail-fast exit mapping) + the parser accepting bare `--headless`; TTY guard with the pipe hint. | `cargo test` (parser + pure helpers) + full gates. |
| **WI.2** | Live on the rig: a piped two-turn session proves same-session continuity (turn 2 references turn 1), and a piped skill-turn + follow-up pair fires Strop's `retro-folded` breadcrumb on the next reflection tick — closing the one unproven Strop path. | Journal breadcrumb + session evidence; findings folded back here. |

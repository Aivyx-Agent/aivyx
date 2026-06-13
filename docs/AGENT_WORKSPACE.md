# Agent Personal Workspace — the agent's own room (Chapter O)

> **Status:** design contract. This is the spec Chapter O scaffolds from
> (mirrors `docs/ACCESS_LEVELS.md` / `docs/HEADLESS_MODE.md`).
>
> Aivyx can put information in two places today: **memory** (`memory.*` —
> discrete facts for semantic recall) and the **operator's `fs_root`** (the
> shared work area, access-level-controlled by Chapter N — but those are the
> *operator's* files, and `fs_root` may be a narrow sandbox or not granted).
> There is no place that is the **agent's own**.
>
> **Chapter O** gives the agent a dedicated, always-available, document-shaped
> workspace it owns — for its thoughts, ideas, plans, and multi-file projects —
> plus the ability to **journal proactively**. It is the third leg of the stool,
> complementary to the other two and independent of the access level.

---

## 0. Decisions (locked at scope time)

| Decision | Choice | Why |
|---|---|---|
| Shape | **Dedicated filesystem workspace** | Free-form, multi-file, hierarchical — fits "Projects" and evolving plans, which memory's flat topic-keyed recall cannot. Memory stays as-is for recall facts. |
| Location | **Always-available private dir** (`~/.aivyx/workspace/`) | The agent ALWAYS has its own space — even at `access level = sandbox`. Independent of `fs_root`; cleanly separate from the operator's files. |
| Autonomy | **Proactive journaling** | The agent periodically reflects on recent activity and writes to its journal on its own — not only when asked. Bounded (at most once per interval, only when there's activity). |
| Reach | **The workspace only** | `workspace.*` tools are rooted at the workspace dir and contained there (the fs.rs fence). They never touch `fs_root` or anywhere else; they are NOT a way around the Chapter N access boundary. |
| Safety posture | **No confirm-first** | It is the agent's own contained scratch space, not operator files — deleting its own note is not dangerous. Still capability-gated + on the HMAC audit chain. |

---

## 1. The three-way split

| Surface | Shape | For | Controlled by |
|---|---|---|---|
| **memory** (`memory.*`) | flat, topic-keyed, semantically searchable | *facts to recall* ("Julian prefers X") | always on |
| **operator `fs_root`** (`fs.*`) | the operator's real directories | *shared work on the operator's files* | Chapter N access level |
| **agent workspace** (`workspace.*`) | free-form files + dirs the agent owns | *the agent's own thoughts / plans / projects* | always on; this chapter |

Recall a fact → memory. Edit the operator's file → `fs.*`. Draft your own plan,
keep a journal, scaffold your own project → the workspace.

---

## 2. The workspace

- **Default path:** `~/.aivyx/workspace/` (sibling of the existing
  `~/.aivyx/tool-processes/`). Resolution: `AIVYX_WORKSPACE` env → `[workspace]
  path` → the default.
- **Provisioned on startup**, idempotently — created if absent and seeded with a
  `README.md` (what this space is) and a light structure the agent is told about:

  ```text
  ~/.aivyx/workspace/
    README.md        ← what this space is, for the agent
    journal/         ← dated entries (manual + proactive)
    ideas/           ← sketches, half-thoughts
    plans/           ← plans the agent drafts and revises
    projects/        ← the agent's own multi-file projects
  ```

- **Transparent to the operator:** a real directory; `aivyx workspace ls / cat`
  lets the operator peek at what the agent is thinking and planning.

---

## 3. Tools (`workspace.*`)

Rooted at the workspace, reusing the `fs.rs` containment fence (`lexical_resolve`
+ canonicalize-parent). Registered unconditionally and held by the default role
(in `backcompat_floor`), so they are available regardless of `fs_root`/access
level.

| Tool | Input | Does |
|---|---|---|
| `workspace.read` | `{ path }` | read a file in the workspace |
| `workspace.write` | `{ path, content }` | write/overwrite a file in the workspace |
| `workspace.list` | `{ path? }` | list the workspace (or a subdir) |
| `workspace.delete` | `{ path }` | delete a file / empty dir in the workspace |
| `workspace.note` | `{ category?, content }` | append a timestamped entry to `journal/<date>.md` (or `<category>.md`) — the easy "jot a thought" primitive |

---

## 4. Proactive journaling

A re-arming background task (sibling of `reflection_scheduler` /
`reminder_driver`) that, every `interval_secs`:

1. Summarizes recent activity over a lookback window (`summarize_recent_outcomes`
   of the audit log) — the same machinery reflection uses.
2. If there was activity, fires a journaling turn
   (`TriggerDispatch::fire(TriggerSource::Reflection, <journaling prompt>, …)`)
   whose system prompt tells the agent to append a brief journal entry and record
   any ideas / plans worth keeping, using the `workspace.*` tools.
3. If idle (no activity in the window), does nothing — no "nothing happened"
   entries, no wasted LLM spend.

Gated by `[workspace.journaling] enabled` (default true) + `interval_secs`.

---

## 5. Config — `[workspace]`

```toml
[workspace]
enabled = true                 # the whole subsystem; false ⇒ as today
path = "~/.aivyx/workspace"     # optional override

[workspace.journaling]
enabled = true
interval_secs = 21600           # 6h; the proactive journaling cadence
```

Absent `[workspace]` ⇒ defaults (enabled, default path, journaling on).
`enabled = false` ⇒ no tools, no provisioning, no journaling — byte-clean opt-out.

---

## 6. Phase plan

| Phase | Deliverable |
|---|---|
| **O.0** | This design contract. |
| **O.1** | `[workspace]` config + path resolution + idempotent startup provisioning. |
| **O.2** | `workspace.*` tools (reusing fs.rs containment), registered always-on. |
| **O.3** | System-prompt awareness of the workspace (path + structure + intent). |
| **O.4** | `aivyx workspace ls / cat / path` operator-visibility command. |
| **O.5** | Proactive journaling background task. |
| **O.6** | Docs, example config, memory note. |

---

## 7. Invariants

- **Always available.** The workspace does not depend on `fs_root` / the access
  level — even a sandboxed agent has its private notebook.
- **Contained + audited.** `workspace.*` stays within the workspace root and
  rides the capability + HMAC audit chain like every other tool. Not a bypass of
  the Chapter N boundary.
- **The agent's own, transparent to the operator.** Separate from operator files;
  the operator can always inspect it.
- **Opt-out clean.** `[workspace] enabled = false` ⇒ behaves exactly as before.
- **Proactive but bounded.** Journaling fires at most once per interval, only when
  there's recent activity; off when disabled.

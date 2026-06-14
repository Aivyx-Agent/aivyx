# Frontend & Brand — the Stitch design system (Chapter R)

> **Status:** design contract + foundation. This is the spec Chapter R builds
> from (mirrors `docs/ACCESS_LEVELS.md` / `docs/LOCAL_FIRST_RUN.md`).
>
> Aivyx has a mature visual identity — **"Stitch"** (*The Neon Cartographer*) —
> defined in the brand repo (`aivyx-brand/`: `brand-guidelines.md`,
> `design-tokens.md`, 23 Stitch mockups). But the shipped web app
> (`crates/aivyx-web`) was a minimal two-tab page with ad-hoc inline CSS. Chapter
> R **locks Stitch into the Agent's frontend**: real tokens, self-hosted fonts, a
> proper app-shell, a reusable component kit, and a reskin of the existing screens
> to match the mockups — while keeping the data flow and local-first guarantees
> untouched.

---

## 0. The brand in one paragraph

**The Neon Cartographer.** Intelligence that maps unseen territory with warmth
and precision. Deep **midnight** surfaces layered into hierarchy (void → base →
surface → raised → elevated → float), grounded by **warm amber** CTAs and a
**cyber-purple** interactive core, with a **warm-tan** tertiary. No hard 1px
section lines — boundaries come from surface-tier shifts and **ghost borders**
(5–10% opacity). Type is a dialogue between human and machine: **Space Grotesk**
(display), **Inter** (body), **JetBrains Mono** (logic/labels). The mark is a
single **candle** — warmth, light, privacy: *a flame in your own kitchen, not a
searchlight in the cloud.* Voice: authoritative, precise, dry. **Tagline: "Your
AI. Your Machine. Your Rules."**

The single source of truth for every value is `aivyx-brand/design-tokens.md` and
`aivyx-brand/brand-guidelines.md`. The frontend transcribes them — it never
invents colors.

---

## 1. Surface = the Agent (Studio). What's in scope.

The Aivyx ecosystem has several surfaces. This contract — and Chapter R — cover
**only the Studio (the agent app)**, the Dioxus→WASM client served by the daemon
at `:7843`.

| Surface | What it is | In this chapter? |
|---|---|---|
| **Studio** | The agent app (chat, missions, dashboard, settings) | ✅ Yes |
| **Genesis** | First-run setup wizard (`aivyx init`) | Roadmap (informs the look) |
| **Unlock** | Vault passphrase screen | Roadmap |
| **TUI** | The ratatui terminal interface | ❌ Later pass |
| **Creator** | Node-based visual flow / agent builder | ❌ Separate product |
| **Nexus** | The agent social network | ❌ Separate product |
| **Marketing** | The public website | ❌ Out of scope |

Delivery is **web-first**: the existing `aivyx-web` bundle, embedded into the
daemon and served on localhost. A Tauri/desktop shell can wrap the *same* app
later; it is not built here.

---

## 2. App-shell layout

The Studio is a classic command-center shell, driven by the layout tokens
(`--sidebar-width: 220px`, `--status-height: 36px`, optional `--tray-width: 300px`):

```
┌───────────────────────────────────────────────────────────┐
│ Topbar:  ▌AIVYX   title / search        ● daemon   ☼ theme │
├──────────┬────────────────────────────────────────────────┤
│ Sidebar  │                                                 │
│ (220px)  │   Main view                                     │
│ logomark │   (Missions · Chat · …)                         │
│  ▸ nav   │                                                 │
│  items   │                          [optional context tray]│
├──────────┴────────────────────────────────────────────────┤
│ StatusBar (36px):  mono daemon/agent status · model · …    │
└───────────────────────────────────────────────────────────┘
```

- **Sidebar** — logomark + nav. Items reflect *real* daemon capabilities; roadmap
  items render **disabled ("soon")**, never as half-built panes.
- **Topbar** — brand/title, connection status dot (`beacon` pulse when live),
  light/dark toggle.
- **StatusBar** — `label-tech` mono line: daemon connection, agent, model.

---

## 3. Studio screen inventory (mapped to daemon capabilities)

| Nav item | Maps to | State |
|---|---|---|
| **Command** | dashboard: stat cards + active missions + live audit-trail feed + agent status | ✅ Live (Ch. S — the default landing view) |
| **Missions** | `team.run` goal→plan→gated execution (Nonagon, Ch. L) | ✅ Live, reskinned |
| **Chat** | single-agent turn loop + streamed events + gate | ✅ Live, reskinned |
| **Teams** | Nonagon roster / vertical packs | Roadmap |
| **Agents** | persona / soul / profile editor: direct Profile write + persona-governance loop (proposals + revert) — see §9 | ✅ Live (Ch. V) |
| **Memory** | self-learning memory browser: topics + entries + search (graph viz later) | ✅ Live (Ch. T) |
| **Documents** | workspace + fs_root browser | Roadmap |
| **Settings** | the first config **write** surface: access level (confirm-first) + budgets editable; provider/model read-only — see §8 | ✅ Live (Ch. U) |
| **Voice** | the voice channel | Roadmap |

The reference mockups for the locked look: `aivyx-brand/assets/stitch/`
`aivyx_command_center`, `aivyx_missions_orchestration`, `the_terminal`.

---

## 4. The mechanisms (how Stitch lands in the bundle)

1. **Token layer.** `crates/aivyx-web/assets/stitch.css` — the full token set as
   `:root` custom properties (dark default) + `[data-theme="light"]` overrides,
   transcribed verbatim from `design-tokens.md`, plus base styles: the `bg-depth`
   gradient canvas, `.label-tech`, glass panel/card/header, ghost separators, and
   the animation keyframes — all gated behind `prefers-reduced-motion`.
2. **Self-hosted fonts.** `assets/fonts/*.woff2` (Space Grotesk, Inter, JetBrains
   Mono — all OFL) with local `@font-face`. **No Google-Fonts CDN** (the mockups'
   CDN `<link>`s are prototype-only and break offline use).
3. **Brand icons + logos.** The `aivyx-brand/icons/` stroke SVG set + `icons.css`,
   and the logomark/wordmark/favicon, bundled as assets. **No Material Symbols
   CDN.**
4. **Wiring.** Dioxus `asset!()` + `document::Stylesheet`/`Link` in the rsx head;
   the inline `STYLE` const is deleted. `just build-web` (`dx bundle`) emits these
   into `dist/`, `aivyx-channel/build.rs` embeds them, `web_ui.rs` serves them —
   so everything is offline and local-first.

---

## 5. Invariants

- **Local-first / offline.** Fonts, icons, CSS are all self-hosted in the bundle;
  the served app makes **zero external requests**.
- **Stitch is the single source of truth.** Every color/space/shadow comes from
  `design-tokens.md`; no ad-hoc values. The inline `STYLE` const is gone.
- **Presentation-only.** The reskin never touches the `aivyx_ipc` data flow, the
  WebSocket task, or daemon behavior. `/classic` stays intact.
- **Studio only.** Creator, Nexus, marketing, the TUI reskin, and any
  Tauri/desktop shell are out of scope; roadmap screens are disabled nav, not
  stubs.
- **No hard lines.** Section boundaries use surface-tier shifts or ghost borders,
  never 100%-opaque 1px rules.

---

## 6. Phase plan

| Phase | Deliverable |
|---|---|
| **R.0** | This design contract. |
| **R.1** | Stitch asset layer — `stitch.css` tokens, self-hosted fonts, brand icons/logos, wired via `asset!()`. |
| **R.2** | App-shell — Sidebar + Topbar + StatusBar from the layout tokens. |
| **R.3** | Component kit — Button / Card / Input / Chip / LabelTech / StatCard / StatusDot. |
| **R.4** | Reskin Missions (orchestration look) + Chat (terminal look). |
| **R.5** | Build the bundle, live-verify at `:7843`, docs/screens, memory. |

**Status: R.0–R.5 complete and verified served.** The Studio app renders the
Stitch shell (Sidebar + Topbar + StatusBar), the reskinned Missions + Chat
views, self-hosted fonts and brand icons — all from the daemon's embedded
bundle with **zero external requests** (verified: the WASM requests the hashed
asset paths, the daemon serves each `200`, the served CSS carries the full token
set in both themes). The built bundle is committed at `crates/aivyx-web/dist/`
so a plain `cargo build` and the release embed it without a wasm toolchain;
**regenerate it with `just build-web` after any frontend change** (needs a
prebuilt `dx` 0.6.x binary — building dioxus-cli from source currently fails on
a pinned `swc`/`serde::__private` conflict). A future `dx bundle` step in the
release CI would remove the need to commit the artifact.

---

## 7. Out of scope (the follow-ons)

The **Memory graph** *visualization*, **Teams/Agents** screens, the
**Genesis wizard** + **Unlock** screens, the **TUI** Stitch reskin, and a
**Tauri/desktop** shell are all future work — they build on this foundation.
(The **Command-Center dashboard** landed in Chapter S; the **Memory browser** —
topics/entries/search — in Chapter T; the **Settings** write surface is Chapter
U, scoped in §8.) **Creator** and **Nexus** are separate ecosystem products with
their own contracts.

---

## 8. Settings — the first config write surface (Chapter U)

Chapters R/S/T were **read-only** and added **no daemon API** — they painted
existing IPC. **Settings is the first screen that writes.** It lets the operator
read and change a deliberate, safe subset of `aivyx.toml` from the Studio. This
section is the contract Chapter U builds from; it intentionally breaks the
"no new daemon API / read-only" invariant (that is the point of the chapter)
while holding every safety invariant Aivyx already guarantees.

### 8.1 Three hard facts that shape the screen

1. **The daemon does not hot-reload config.** Access level, budgets, provider,
   model — all are parsed **once at launch** (access level is load-time;
   `aivyx access`: "takes effect on the next daemon start"). A write from the web
   UI therefore **cannot apply live**. The screen is honest about this: every
   successful write returns `restart_required` and the UI shows a persistent
   banner — *"Saved to aivyx.toml — restart the daemon to apply:
   `aivyx daemon stop && aivyx daemon run`."* No self-restart (too invasive).
2. **The daemon doesn't retain the config-file path.** `DaemonConfig` holds
   parsed sub-structs, not the path to `aivyx.toml`. The write path adds a
   `config_toml_path` to `DaemonConfig`, threaded from `aivyx daemon run`, so the
   daemon can both **re-read** the on-disk values (to populate the form) and
   **rewrite** the right file.
3. **The write logic already exists** in `aivyx access set` (a `toml_edit`
   section-rewrite that preserves every other section, sets `0600`, drops a stale
   `[access] root`, and confirms expanded levels). Chapter U **factors it into a
   shared `aivyx-config` helper** so the CLI and the daemon write config
   identically — never two divergent writers.

### 8.2 Editable scope (v1)

| Section | v1 | Why |
|---|---|---|
| **Access level + root** | ✅ Editable, **confirm-first** on expansion | The flagship; security-sensitive (Ch. N). Mirrors `aivyx access set`. |
| **Budgets** (`per_run_usd`, `per_day_usd`, `on_exceeded`, `alert_at`) | ✅ Editable | Low-risk numeric caps (Ch. K). |
| **Provider / model / num_ctx** | 👁 **Read-only** + "change via `aivyx init`" | Editing risks a daemon that won't start (bad model) and touches API keys in the encrypted store/env — out of v1. |
| **Profile** | ❌ A future Agents/Persona screen | Already served read-only by `GetProfile`; editing is its own surface. |

### 8.3 New IPC (request/response — fits the existing query pattern)

- **`GetSettings`** → `SettingsSnapshot { access_level, fs_root,
  confirm_destructive, provider, model, num_ctx, budget{…}, embeddings_available }`
  — populates the form (none of this is queryable today).
- **`SetAccessLevel { level, root, confirm }`** — the daemon enforces
  `is_expanded() ⇒ confirm == true` **server-side** (the confirm-first gate is not
  just a UI nicety), applies the same root rules as the CLI.
- **`SetBudget { per_run_usd, per_day_usd, on_exceeded, alert_at }`**.
- Both writes: rewrite the toml section via the §8.1(3) helper → append a new
  **`ConfigChanged`** audit event → respond with the fresh snapshot +
  `restart_required: true`.

### 8.4 Invariants

- **Localhost trust boundary.** The Studio is served on localhost only — the same
  boundary the CLI already writes config from — so this adds no new attack
  surface. It is *not* a remote admin panel.
- **Confirm-first survives.** Chapter N's confirm-before-expanding-access posture
  is preserved as a **UI confirm modal _and_ a server-side `confirm` flag** the
  daemon refuses to bypass.
- **Every change is audited.** Writes append a signed `ConfigChanged` entry to the
  HMAC audit chain — the same chain the Command Center's feed renders.
- **Writes preserve the file.** Section-scoped `toml_edit` rewrites at `0600`;
  comments and unrelated sections (secrets, providers, triggers) are untouched.
- **Studio only / local-first.** Same scope boundary and Stitch/offline rules as
  R–T; `/classic` stays intact.

### 8.5 Phase plan

| Phase | Deliverable |
|---|---|
| **U.0** | This contract (§8). |
| **U.1** | Shared `aivyx-config` write helpers + `config_toml_path` on `DaemonConfig` (threaded from `aivyx daemon run`); CLI refactored onto the helper (no behavior change). |
| **U.2** | `aivyx-ipc`: `GetSettings` + `SetAccessLevel` + `SetBudget` + `SettingsSnapshot`, with wire-compat round-trip tests. |
| **U.3** | Daemon handlers: snapshot read; validate → rewrite → **`ConfigChanged`** audit → respond with `restart_required`. (Adds an `AuditEvent` variant → updates the e2e event-count assertions; full suite.) |
| **U.4** | Web UI: `View::Settings` live — access selector + confirm modal, budget inputs, read-only provider/model card, restart banner; `ws_task` arms; `stitch.css`. |
| **U.5** | Build the bundle, live-verify in a real browser (read settings, set a budget, change access via the modal, see the restart banner, confirm the toml is rewritten with other sections preserved + an audit entry), docs + memory, push. **Done:** wasm serves byte-identical/untruncated; IPC probe proved `GetSettings`→`SetBudget`→`SetAccessLevel` (confirm-first refusal then apply), toml rewritten preserving all sections, two `ConfigChanged` audit entries, chain intact. |

---

## 9. Agents — the identity editor (Chapter V)

Settings (Ch. U) opened the first config-write surface. **Agents** is the
second, and the one that touches the product's core identity: the operator's
**Profile** and the agent's self-learned **Persona**. It is deliberately *two
different write models stitched into one screen*, because the two halves are
governed differently and that difference is a feature, not an accident.

### 9.1 The three layers (what is editable, and how)

| Layer | What it is | Edit model | Liveness |
|---|---|---|---|
| **Profile** | operator-**declared** identity — the `[profile]` table in `aivyx.toml` (`assistant_name`, `operator_profile`, `communication_style`, `primary_use_cases[]`, `behavioral_preferences[]`, `behavioral_constraints[]`) | **direct write** — surgical `toml_edit` rewrite of `[profile]`, exactly the Ch. U pattern (`aivyx profile edit` is the CLI twin) | **load-time** → `restart_required` (same as Settings; Profile shapes `assemble_session_prompt` at startup) |
| **Persona** | the agent's **self-learned** adaptations — an append-only, HMAC-signed **delta chain** (behavioral prefs, learned context, communication adaptations, character traits, relationship milestones) | **never free-edited.** The operator *governs* it: resolve agent **proposals** (approve / approve-with-edit / reject) and **revert** deltas. The chain's integrity is the point. | **live** — the daemon recomputes shared runtime state on resolve/revert, so the next turn picks it up (no restart) |
| **Soul / Identity** | the combined Profile+Persona **export bundle** (`aivyx identity export`) | portability, not editing | n/a — **deferred** (a later read-only export button) |

The screen never lets the operator hand-write persona deltas. That asymmetry —
**you declare your Profile; the agent proposes its Persona and you gate it** — is
the self-learning contract (PRODUCT.md P13/P14) made visible.

### 9.2 What already exists (reuse, do not rebuild)

Persona is a mature, gated subsystem; **almost all of its write IPC already
ships** and is daemon-tested (the `aivyx persona` CLI + `/classic` use it):

- **Read:** `GetProfile` → `ProfileSummary`; `GetEffectivePersona` →
  `EffectivePersonaSummary`; `ListPersonaDeltas` → `[PersonaDeltaSummary]`;
  `ListPersonaProposals` / `GetPersonaProposal` → `[PersonaProposalSummary]`.
- **Write (existing `FrontendMessage`):** `ResolvePersonaProposal { proposal_id,
  resolution: Approve | ApproveWithEdit { edited_op } | Reject { reason } }` →
  `PersonaProposalResolved`; `RevertPersonaDelta { target_delta_id }` →
  `PersonaRevertResolved`. Both recompute runtime state (live).

So the **only new daemon API this chapter adds is the Profile direct-write
path** — `SetProfile` + a `write_profile_section` helper in `config_write.rs`
(joining `write_access_section` / `write_budget_section` from U.1). Everything
persona is wiring existing IPC into the Studio, the way Chat wired the existing
turn loop.

### 9.3 New IPC (the Profile half only)

- **`SetProfile`** `{ assistant_name?, operator_profile?, communication_style?,
  primary_use_cases?: Vec<String>, behavioral_preferences?: Vec<String>,
  behavioral_constraints?: Vec<String> }` → `ProfileApplied { profile:
  ProfileSummary, restart_required: true }`. Absent scalars clear the key;
  absent lists leave them untouched vs. an explicit `[]` that clears — TBD in
  V.2, matched to `write_budget_section`'s clear-on-None convention.
- Reuses `config_toml_path` (added in U.1) and `ConfigWriteError` → stable
  `QueryError` codes (`map_config_write_error`).
- Writes append an `AuditEvent::ConfigChanged { section: "profile", summary }`
  (the U.3 variant — no new audit variant, no new count-assertion churn).
- **Active vs on-disk (post-V audit).** The daemon holds the Profile as an
  immutable boot-time `Arc<Profile>` — it is **load-time**, so the running agent
  keeps using the boot values until a restart (the Persona, by contrast, is a
  live `Arc<RwLock>` and updates next-turn). `GetProfile` therefore carries a
  `from_disk` flag: the editor seeds with `from_disk = true` (the on-disk
  `[profile]`, i.e. *what it writes*) so a save-before-restart then reload shows
  the pending values and never clobbers them; the Command-Center name chip uses
  `from_disk = false` (the running snapshot). The flag is `#[serde(default)]`
  (false), so the running-state meaning is wire-unchanged for older clients.

### 9.4 Invariants

- **Profile writes preserve the file.** Section-scoped `toml_edit` at `0600`;
  `[agent]`/`[fs]`/`[access]`/`[budget]`/secrets untouched — same guarantee as U.
- **Persona integrity is never bypassed.** The web UI cannot append a raw delta;
  it can only resolve a proposal or revert via the existing signed-chain paths.
  Approve-with-edit carries an `edited_op` the daemon re-validates and re-signs.
- **Honest liveness.** Profile edits show the restart banner; persona
  resolve/revert show "applied — effective next turn" (no banner).
- **Every change is audited.** Profile → `ConfigChanged`; persona → the existing
  `PersonaProposalResolved` / `PersonaRevertResolved` audit entries.
- **Studio only / local-first / Stitch.** Same scope + offline + token rules as
  R–U; `/classic` intact.

### 9.5 Phase plan

| Phase | Deliverable |
|---|---|
| **V.0** | This contract (§9). |
| **V.1** | `write_profile_section` in `aivyx-config/config_write.rs` (validate + `[profile]` toml_edit rewrite, 0600); unit tests alongside the U.1 helpers. |
| **V.2** | `aivyx-ipc`: `SetProfile` + `ProfileApplied` (round-trip tests); daemon handler (validate → rewrite → `ConfigChanged` audit → `restart_required`). Confirm the existing persona read+resolve+revert handlers cover what the web screen needs. |
| **V.3** | Web UI part 1 — `View::Agents` + the **Profile editor** (form over the six `[profile]` fields, list add/remove, save → confirm → `ProfileApplied`, restart banner); `ws_task` arms; `stitch.css`. |
| **V.4** | Web UI part 2 — the **Persona governance** panel: Effective Persona viewer + pending **proposals** (approve / edit / reject) + **delta chain** with revert, over the existing IPC. "Effective next turn" notices. |
| **V.5** | Build the bundle, live-verify in a real browser (edit Profile → restart banner + toml rewritten + audit; resolve a seeded proposal → persona updates live; revert a delta), docs + memory, push. **Done:** wasm serves byte-identical/untruncated; IPC probe proved `GetProfile`→`SetProfile`→`ProfileApplied` (six fields, `restart_required`), `[profile]` rewritten **in-place** preserving all sections, a shape-only `ConfigChanged` audit entry, the persona read path (effective/proposals/deltas) returns well-formed empty states on a fresh daemon, and the resolve/revert write path is wired (typed `ok:false` on bogus ids). Seeding a real proposal needs LLM reflection turns, so the resolve/revert *UI* actions are verified against the live error path rather than an approved delta. |

# UI Modernization pass (POLISH_WAVES.md sub-project 6) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 6 bundles 4 findings, all UI-only
(no data-correctness gaps): the Command Center dashboard reads as
"flat and boxy, similar to every other agentic dashboard"; the Memory/
Wiki knowledge graphs are "messy and unintuitive"; a redeployed WASM
bundle needs a manual browser reload with no hint that one's due; and
Documents renders every file (including markdown) as raw monospace
text.

Direction agreed before this doc was written: adopt Aivyx's own
existing Stitch design-system visual language (depth/glass, warm
amber accent, subtle motion) — explicitly **not** the Stitch mockups'
sci-fi command-center copy style, fictional data density, or
fabricated features. Each item below was grounded directly against
the current code, not assumed from the tracking doc's prose.

## Scope

**In** — all 4 items below. **Out** — the config-write surface area
(sub-project 7, sequenced after this pass specifically so its new
screens are built directly in the refreshed visual language, never
restyled) and anything the missions-polish chapter (sub-project 5)
already closed.

One design doc, one implementation plan, one SDD execution.

## A. Command Center dashboard restyle

**Finding:** `.card`/`.glass-card` (`crates/aivyx-web/assets/
stitch.css:293-302`) define background, `backdrop-filter: blur(12px)`,
and a border — but no `box-shadow` at all. `aivyx-brand/
brand-guidelines.md`'s own Shadow System table assigns `shadow-md` →
"Cards" and `shadow-ambient` → "Page sections", but in the real
stylesheet those two tokens are applied nowhere except overlay chrome
(`.palette`, the mobile `.sidebar` drawer at `max-width: 860px`) —
`--shadow-ambient` and `--shadow-wick` are otherwise **dead
variables**, defined and never referenced. The Stitch mockup itself
(`aivyx-brand/assets/stitch/aivyx_command_center/code.html`) pairs its
own `.glass-panel` (byte-identical `rgba`/blur values to Studio's real
`.glass-card`) with a second `.ambient-glow` utility class carrying
exactly `--shadow-ambient`'s value. This is the entire "flat and
boxy" complaint: glass without the depth cue the brand system's own
spec calls for alongside it.

`CommandPanel` (`crates/aivyx-web/src/main.rs:1371`) and its skeleton
counterpart already use `.glass-card` throughout (`.stat-card`,
`.dash-mission` via `DashMissionRow`, `.routine-row` via
`RoutineRow`), as does every other screen that uses `.panel`/
`.glass-card` — Missions, Memory, Guide, Schedules. The fix is CSS-only
and app-wide by construction, with Command Center as the visible
headline since it's the first screen an operator sees.

**Design:**
1. Add `box-shadow: var(--shadow-md);` to the existing `.glass-card`
   rule and `box-shadow: var(--shadow-ambient);` to `.panel`
   (`stitch.css:375`) — the two-tier depth the brand guide already
   documents ("inner cards sit atop sections, which sit on the global
   canvas").
2. Add `.fade-in` (already defined at `stitch.css:341`, currently
   unused on the dashboard) to `CommandPanel`'s `.stat-row` and each
   `.panel` on first paint.
3. Add a hover-glow to `.stat-card`, reusing the existing
   `.icon-btn:hover { box-shadow: var(--shadow-glow); }` pattern
   (`stitch.css:255`) rather than inventing a new rule.
4. No markup restructuring, no new components, no layout rework —
   `CommandSkeleton` (`main.rs:1472`) already mirrors the real
   layout exactly, so it needs the identical CSS changes (automatic,
   since it shares the same classes) and no code changes of its own.

**Explicitly rejected:** a from-scratch dashboard layout (asymmetric
grid, larger tiles, icon accent bars) inspired more heavily by the
mockup's specific composition. The root cause is a missing depth cue
the brand system already specifies, not a structural layout problem —
a full rework would be scope creep the finding doesn't support.

## B. Version-mismatch reload hint

**Finding:** no app-version signal crosses the daemon↔Studio
WebSocket today. Studio's footer (`main.rs:1362`) shows only its own
compiled-in `env!("CARGO_PKG_VERSION")` — nothing to compare it
against. A similar-but-unrelated precedent exists on the *other* wire
protocol: `DaemonMessage::ProtocolAccepted { version }` /
`ProtocolRejected { supported }` (`aivyx-ipc/src/protocol.rs:2162`,
Phase 41 Task 5) is a hard version-gate handshake for the raw
Unix-socket protocol used by CLI/TUI/channel adapters — Studio's WS
bridge (`aivyx-channel/src/daemon_server.rs`) never participates in
it and has no equivalent of its own.

**Design:**
1. Add `DaemonMessage::ServerInfo { web_version: String }` to
   `aivyx-ipc/src/protocol.rs`'s `DaemonMessage` enum, sent once by
   the daemon immediately after a Studio WS connection is accepted
   (`daemon_server.rs`, alongside wherever the connection's initial
   state currently gets pushed). `web_version` is `aivyx-web`'s own
   crate version — i.e. `env!("CARGO_PKG_VERSION")` read at the point
   the daemon's build embeds `aivyx-web`'s `dist/` (confirm the exact
   mechanism during planning: whether this is baked in via a
   `build.rs` reading `aivyx-web/Cargo.toml`, or whether `aivyx-web`'s
   version already tracks the workspace version via `version.workspace
   = true` — if the latter, `env!("CARGO_PKG_VERSION")` read from
   *any* workspace crate's build gives the same string, simplifying
   this to no new plumbing at all).
2. Studio's `read_task`/message-handling match arm compares
   `web_version` to its own `env!("CARGO_PKG_VERSION")` (the constant
   already used at `main.rs:1362`). On mismatch, set a new
   `AppUi`-level (or equivalent existing top-level UI state) flag and
   render a small dismissible banner/toast: "A new version of Aivyx
   Studio is available — reload to update." A manual dismiss hides it
   for the session; no polling or repeat-nagging.
3. **Precise scope statement** (for the spec's own clarity, not just
   this design doc): this catches a *redeployed Studio bundle* — i.e.
   `aivyx-web`'s dist changed since the currently-open tab loaded it.
   It does not and cannot detect an unrelated daemon-only rebuild that
   didn't touch `aivyx-web`, since the version compared is
   `aivyx-web`'s own, not the daemon binary's.

**Explicitly rejected:** client-side polling of a `/version` endpoint
or re-fetching `index.html`'s asset hash. The app already has a
live WS connection and an established one-shot-message-on-connect
idiom (`ProtocolAccepted`) to reuse the spirit of; reinventing version
signaling via HTTP polling adds a second mechanism for no benefit.

## C. Memory/Wiki graph-view cleanup

**Finding:** `MemoryGraph` (`main.rs:4154`, Memory topics) and
`LatticeGraph` (`main.rs:3980`, Wiki entities) both consume the same
`compute_layout` function (`main.rs:4082`, a from-scratch
Fruchterman-Reingold force-directed layout: spiral-seeded by a stable
per-topic hash, 220 iterations of repulsion/attraction, clamped to a
fixed `GRAPH_W × GRAPH_H` SVG canvas) and share the same rendering
flaw: every node's text label is placed unconditionally at
`(cx, cy + r + 11.0)` with **zero collision avoidance** against
neighboring labels. `LatticeGraph` compounds this further — it also
places a `predicate` text label at every directed edge's midpoint
(`main.rs:4031`), a second, denser source of overlapping text. At any
real node density, labels smear into an unreadable mess. This matches
the doc's own framing precisely: "data is correct, presentation needs
layout/readability work" — the FR layout math itself isn't the
problem.

Both components already share the layout engine (good existing
DRY) — the fix lands once in shared code and both screens inherit it.

**Design:**
1. In `compute_layout` (or a thin wrapper around its output), add a
   simple post-layout label-collision pass: for labels whose bounding
   boxes would overlap (based on approximate text width from label
   length × a fixed char-width constant, no real text-measurement
   API needed), alternate placement above/below the node by parity,
   and if still colliding after that, skip drawing the label at rest
   (see point 3).
2. Add pan/zoom to the SVG canvas: a `viewBox` that responds to a
   scale/translate state (two new signals: zoom level, pan offset),
   wheel-to-zoom and drag-to-pan handlers on the `svg` element. This
   lets an operator zoom into a dense cluster instead of always
   fit-to-canvas.
3. Past a node-count threshold (exact number TBD during planning by
   testing readability at a few counts — starting guess: >25 nodes),
   switch from always-on labels to hover/selection-only: a node's
   label renders only when hovered or when it's the selected node
   (`MemoryGraph` already threads `on_select`/has a notion of the
   active topic; `LatticeGraph` currently has neither — needs a local
   hover-tracking signal added).
4. Fade low-weight edges further (lower the existing
   `opacity: 0.12 + frac * 0.5` floor) so the dominant structure reads
   more clearly at a glance, without removing weak edges from the data
   entirely.

**Explicitly rejected:** replacing the hand-rolled FR layout with an
external graph-layout crate, or a clustering/connected-components
pre-pass. Both are real ways to attack "messy," but they're
layout-algorithm rewrites for a problem that — per the finding above —
lives in rendering, not positioning. Try the presentation fix first;
if it proves insufficient, that's grounds for a follow-up chapter, not
a decision to make speculatively here.

## D. Documents markdown rendering

**Finding:** `FileViewer` (`main.rs:7336`) renders any file's content
as `pre { class: "doc-text", "{text}" }` — raw monospace, no markdown
parsing — matching the complaint exactly. `aivyx-web` already depends
on `pulldown-cmark` (`Cargo.toml:42`) and already has a working
markdown→HTML pipeline: `guide::render()` (`guide.rs:117`), used by
`GuidePanel` via `dangerous_inner_html`. Its own doc comment is
explicit about why that's safe: guide pages are "our own committed,
trusted content (never user input)." `pulldown-cmark` passes raw
inline/block HTML straight through to its `html::push_html` output —
so reusing `guide::render` verbatim for Documents (files an agent
wrote via `fs.write`, or arbitrary content under the "Files"
filesystem root) would let embedded `<script>`/event-handler markup
execute inside Studio's own page, with a live WS session to the
daemon. That's a real content-injection escalation path, not a
cosmetic gap — the same risk class `Bulwark`/`fence_untrusted_output`
(`aivyx-core/src/agent.rs`) already exists to guard on the
tool-output-into-model-context side; this is the tool-output-into-
UI-execution-context sibling case, and it's currently unguarded.

**Design:**
1. Add a new function, `render_untrusted_markdown` (in `guide.rs`
   alongside `render`, or a new `markdown.rs` module — decide during
   planning based on which keeps `guide.rs`'s existing scope clean),
   that walks `pulldown_cmark::Parser`'s `Event` stream directly
   instead of handing it straight to `html::push_html`: pass every
   event through unchanged **except** `Event::Html(_)` and
   `Event::InlineHtml(_)`, which are either dropped or re-emitted as
   `Event::Text` (escaped, so a literal `<script>` in the source shows
   up as inert visible text rather than executing or vanishing
   silently — dropping loses information a reader might need to
   debug their own file; escaping doesn't). `guide::render` itself is
   untouched — this is a new, separate function for the untrusted-
   input case, not a modification of the trusted-content path.
2. In the same event-mapping pass, special-case fenced code blocks
   whose info string is `mermaid`: instead of emitting a normal
   `<pre><code>` block, emit `<pre class="mermaid">{escaped
   contents}</pre>` (the markup mermaid.js itself expects to find and
   typeset).
3. `FileViewer`: when `file.content.is_some()` and the file's name
   ends in `.md`/`.markdown`, render via `render_untrusted_markdown`
   inside `dangerous_inner_html` (same `.glass-card`-style container
   pattern `GuidePanel` uses) instead of the raw `pre { "doc-text" }`
   block. `FileViewer` is also editable
   (`editable = file.content.is_some() && !file.binary`) — keep a
   toggle (e.g. a small "Source"/"Preview" switch) so the operator can
   still see and edit the raw markdown text; default to Preview for
   `.md` files, Source for everything else (unchanged behavior).
4. Mermaid rendering: vendor `mermaid.min.js` under
   `crates/aivyx-web/assets/vendor/mermaid.min.js`, bundled via the
   existing `asset!("/assets/...")` idiom every other static asset in
   this crate already uses (fonts, icons, `stitch.css`) — served from
   the daemon's own embedded bundle, no CDN call, matching Aivyx's
   local-first/no-external-network-dependency stance. **Lazy-load**:
   don't reference the asset from the base app shell; the first time
   `FileViewer` renders a `.md` file whose content contains a
   ` ```mermaid ` fence, create a `<script>` element at runtime via
   `web_sys::Document::create_element("script")` +
   `set_src`/`append_child` (dynamically-created script tags execute,
   unlike ones injected via `dangerous_inner_html`, which the browser
   never runs per spec). Once that script's `load` event fires, call
   `mermaid.run()` — the vendored library attaches itself to
   `window.mermaid` as a global (its usual UMD/browser build) — via a
   small `wasm-bindgen`/`js_sys` interop shim (new, this crate has no
   call-into-a-vendored-global-library precedent yet; keep the shim to
   the few lines this needs, not a general JS-interop framework).
   Re-invoke `mermaid.run()` on every subsequent `.md` render that
   contains a mermaid fence (the script itself only needs loading
   once per session — track that with a signal or a check for
   `window.mermaid`'s existence before re-injecting the `<script>`
   tag).

**Explicitly rejected:**
- Reusing `guide::render` verbatim and accepting the injection risk
  as tolerable given Aivyx's single-operator, local-first threat
  model. Aivyx's own established stance (`Bulwark`) treats
  agent-output-re-entering-a-trusted-context as a named risk class
  worth guarding, not a theoretical one to wave off by appeal to the
  operator being the only user — the operator opening a file an agent
  wrote is exactly the scenario where the *agent's* output (which may
  itself be influenced by untrusted content it read) re-enters a
  privileged surface.
- A full CommonMark-to-`rsx!`-elements walker that never uses
  `dangerous_inner_html` at all for Documents. Strictly safer by
  construction, but meaningfully more implementation surface (every
  markdown construct needs its own `rsx!` mapping) for a
  polish-pass-scoped fix when the event-stream-filtering approach
  closes the same hole with far less new code.
- A sanitizer crate (e.g. `ammonia`) instead of the event-stream
  filter. Avoided specifically to sidestep an unverified
  wasm32-unknown-unknown compatibility question for a crate this
  workspace has never built for that target before — the event-stream
  approach needs no new dependency at all.

## Testing

- **A**: CSS-only; no unit test surface. Verify visually via a
  rebuilt `dist/` bundle (screenshot comparison against the
  brainstorming companion's mockup reference is reasonable but not
  required).
- **B**: a `to_view`-style or protocol-level test asserting
  `ServerInfo`'s `web_version` round-trips through
  serialize/deserialize; a pure-function test on the client-side
  mismatch-detection logic (`"1.2.3" != "1.2.4"` → banner shown,
  equal → not shown) — mirrors this file's existing pure-function test
  conventions (`gate_label`-style).
- **C**: a unit test on the new label-collision helper (two nodes
  close enough to collide → alternate placement; far enough → no
  change) and on the hover/selection-only threshold logic, both as
  pure functions independent of the Dioxus runtime where possible —
  matches this codebase's established pattern for `compute_layout`-
  adjacent logic.
- **D**: unit tests on `render_untrusted_markdown` directly (a
  `<script>alert(1)</script>` input renders as escaped visible text,
  not executable markup; a ` ```mermaid ` fence renders as
  `<pre class="mermaid">`; ordinary markdown — headings, tables,
  lists, links — round-trips the same as `guide::render` would for
  the non-HTML-bearing subset) — these are pure string-in/string-out
  tests, no wasm runtime needed. The `<script>`-creation/mermaid.run()
  interop path itself isn't unit-testable outside a browser; confirm
  it manually against a `.md` file containing a real mermaid fence
  during planning's manual-verification step, same as this crate's
  other WS/DOM-interop code paths.
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test
  --workspace --exclude aivyx-desktop`, zero warnings/failures, plus
  `cargo build -p aivyx-web --target wasm32-unknown-unknown` (the
  cheap wasm-compile guard `just check-web` runs) and a rebuilt +
  committed `dist/` bundle per this repo's established convention
  (`rm -rf dist && cp -r ... dist`, never a merging copy; strip `.br`
  files; verify the wasm diff is exactly one add + one delete).

## Out of scope

- The config-write surface area (sub-project 7) — sequenced after
  this pass so its new Schedules/MCP/notify-target screens are built
  directly in the refreshed visual language, never restyled
  afterward.
- Mission-topic-naming discipline (sub-project 5, item D) — still
  open, unrelated to visual design, not reopened here.
- Any change to `compute_layout`'s actual force-directed algorithm,
  or to Concord's memory-graph/knowledge-graph data (`GetKnowledgeGraph`,
  `GetMemoryGraph`) — this chapter only touches presentation of data
  that's already correct.
- A general JS-interop framework for future vendored libraries beyond
  the minimal shim item D's mermaid support needs.
- Full editor-grade markdown authoring (live split-pane preview,
  syntax highlighting in the Source view) — item D adds a read
  preview and keeps the existing plain-text editor for Source; richer
  authoring tooling is a separate, unscoped idea.

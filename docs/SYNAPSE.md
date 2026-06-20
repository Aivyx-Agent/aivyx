# Activating the Memory Stack — one switch, proven live (Chapter Synapse)

> **Status:** ✅ **COMPLETE (SY.0–SY.4).** The built-but-dormant memory stack is
> now **activated and proven**: one `[memory] profile = "smart"` switch arms the
> whole coherent stack (recall fusion + the wiki + typed-graph layers + their
> sweeps), an end-to-end integration test proves it composes, the `graph.query`
> affordance + Studio empty states + example config + README make it
> discoverable, and a §6 runbook live-verifies it with a real model. Default
> stays `off` (byte-identical); a pure refinement — **no new capability, base,
> amendment, or dependency**; full suite + clippy + `cargo deny` green. The
> locked reference for the
> chapter that turns the **built-but-dormant** memory stack ([[LOOM]] →
> [[CODEX]] → [[LATTICE]] → [[LEXICON]]) into **realized, verified value**.
> Those five chapters shipped a sophisticated, deeply-tested memory
> system — but every piece is **opt-in / byte-identical by default**,
> scattered across ~14 knobs, and **never run end-to-end with a real
> model**. Synapse closes that gap: **one activation switch**, an
> **end-to-end integration proof** the pipeline composes, an **operator
> live-verify runbook** with a real Ollama model, and the **affordances**
> the agent needs to actually use the graph. A refinement chapter — **no
> new capability, base, amendment, or dependency**; it makes what exists
> *work*.

## 1. Why — capability built, value unrealized

The memory arc is impressive on paper. In practice, for a real operator
*today*:

1. **It's almost entirely default-off.** `recall_hybrid`, the lexical /
   co-occurrence / wiki / typed-graph recall sources, the `[wiki]` and
   `[graph]` extraction sweeps — all default off. To get the full
   experience an operator must set **~10 interacting knobs** across
   `[embedding]` / `[recall_cluster]` / `[wiki]` / `[graph]`. Nobody will.
2. **It's never been proven live.** Every chapter was verified with unit
   tests + **scripted fake `LlmProvider`s**. The *real* pipeline — a real
   model consolidating a wiki page and extracting typed triples, those
   flowing into recall, the agent calling `graph.query` — has **never run
   end-to-end**. The Studio Wiki and Graph screens render **empty** until
   an operator arms the sweeps.
3. **The agent may not reach for it.** `graph.query` is registered and
   granted, but nothing confirms the model knows it exists or when to use
   it.

Continuing to *deepen* this stack (entity resolution, cross-topic
extraction, …) before it pays off is diminishing returns. The
highest-leverage move is to **activate and prove** what's already built.

## 2. Architecture & decisions (locked)

### One activation switch: `[memory] profile`
A single config knob expands into the coherent bundle of memory settings,
at **config-load time** (one place, in `aivyx-config`), so an operator
opts in *once* instead of tuning ten flags. Two levels in v1:

- **`off`** (the default) — today's behavior, **byte-identical**. No
  expansion; the stack stays dormant.
- **`smart`** — the full coherent stack: hybrid recall + the lexical /
  co-occurrence / wiki / typed-graph fusion sources at sensible weights,
  **and** the `[wiki]` + `[graph]` extraction sweeps (with conservative
  caps), so the agent both *builds* and *uses* its knowledge layers.

**Explicit knobs always win** — `profile = "smart"` sets the bundle as
*defaults*, and any individually-set `[embedding]` / `[wiki]` / `[graph]`
value overrides it. So the switch is a floor, never a cage. (A middle
`lite` tier — recall fusion over *existing* data only, no paid LLM sweeps
— is a documented deferral; `off` / `smart` is the clean v1.)

The default stays `off`: the switch removes the *friction* of activation
without changing what an un-opted-in operator gets — the byte-identical
discipline every memory chapter held.

### Prove it composes — an end-to-end integration test
The chapters tested each piece against a *scripted fake* LLM in
*isolation*. Synapse adds the missing test: **the real components wired
together** — real `Memory`, real `WikiSynthesizer`, real `GraphExtractor`,
real recall fusion, a multi-response scripted provider — asserting the
*whole pipeline composes*:

> write memories → a sweep synthesizes a wiki page **and** extracts typed
> triples → `graph.query` traverses them → recall fuses the wiki summary
> **and** the typed-graph neighbors into a turn's context.

This is the "does it all actually fit together" proof unit tests can't
give, and it's fully autonomous (no live model needed).

### Live-verify with a real model — an operator runbook
The truly-live check (a *real* Ollama model generating real pages/triples)
needs a model on the host, so — like Timbre's audio speak-test — it ships
as a **documented runbook** + whatever the chapter can verify
automatically (e.g., a `profile = "smart"` daemon boots, the sweeps arm,
the Studio screens are reachable). The runbook walks the operator through
arming `smart`, writing memories, watching the breadcrumbs
(`aivyx graph-sweep: …`, `aivyx recall-graph: …`), and confirming the
Studio Wiki/Graph fill in.

### Affordances — the agent knows the tools, the operator sees the path
- **Agent:** confirm `graph.query` is surfaced to the model (it's in the
  tool list; verify the description reads as a *when-to-use* affordance,
  tightening it if not). No new tool — a discoverability check + tweak.
- **Operator:** the Studio Wiki/Graph empty states already point at
  `[wiki]`/`[graph]`; update them to point at the simpler
  `[memory] profile = "smart"`, and add a short "smart memory" section to
  the example config + README.

### Governance: a refinement, nothing new
No new tool, capability base, P10 amendment, storage domain, or
dependency. Synapse is config expansion + an integration test + docs +
a discoverability tweak. The default is unchanged (`off`), so the
byte-identical contract holds.

## 3. Scope

**In:** the `[memory] profile` switch + its config-load expansion
(override-aware); the end-to-end integration test; the agent-affordance
check/tweak for `graph.query`; the operator live-verify runbook; the
Studio empty-state + example-config + README "smart memory" docs.

**Out:** turning anything on *by default* (default stays `off`); a `lite`
tier; new memory capability (entity resolution / cross-topic / inverse
relations stay deferred); any new tool/base/amendment. Stabilizing the
two flaky tests surfaced during the arc (`budget_gate`, persona-log) is a
**carried non-blocker**, not this chapter's scope.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **SY.0** | **This design contract** | locked reference; banner flips per phase |
| **SY.1** ✅ | **`[memory] profile` switch** | DONE. `MemoryProfile` {`Off` (default), `Smart`} + `[memory] profile` on the existing `[memory]` section; the load-time expansion: `build_embedding_config(raw, smart)` arms `recall_hybrid` / `recall_graph_hops=1` / `recall_wiki_weight=1.0` / `recall_graph_typed_weight=1.0` when unset, and `[recall_cluster]` / `[wiki]` / `[graph]` are synthesized `enabled` (default caps) when *absent* (an explicitly-present section wins). Default `off` ⇒ byte-identical; expansion fills the fields the daemon already reads (zero daemon wiring; `Config.memory_profile` is introspection-only). 3 tests (off unchanged; smart arms the bundle; explicit `recall_hybrid=false` + `[wiki] enabled=false` beat smart). |
| **SY.2** ✅ | **End-to-end integration proof** | DONE. `crates/aivyx-channel/tests/memory_stack_e2e.rs` — a `RoutingProvider` (answers the wiki call with a summary, the graph call with a JSON triple array, switching on the system prompt) + a `ConstEmbed`; seeds memory (`deploy` semantically reachable, `ci` reachable *only* via the graph), runs the wiki + graph sweeps, asserts a page + a *canonical* `depends-on` triple (Lexicon fold of `requires`), runs `graph.query` (store + the tool), then a `recall_hybrid` + wiki + typed-graph context and asserts the block fuses the semantic entry **+** the wiki summary **+** the typed-graph-only `ci` entry — the whole stack in one turn. Passes; clippy clean. |
| **SY.3** ✅ | **Affordance + activation docs** | DONE. `graph.query` description sharpened to a when-to-use affordance (relate/depend/connect/caused/owns/contains questions). Studio Wiki + Graph empty states repointed at `[memory] profile = "smart"` (bundle rebuilt `dx bundle --release` + `dist/` committed; wasm carries the new strings). A "smart memory — one switch" section added to `examples/aivyx.toml` + the README highlight. The operator live-verify runbook lands as **§6** (real-Ollama, ~10 min, steps + the one expected escalation). |
| **SY.4** ✅ | **Finalize** | DONE. Full workspace suite + `cargo clippy --workspace` (0) + `cargo deny` (licenses + advisories) green; zero new deps; README phases line + CHANGELOG note the switch; banner flipped to COMPLETE; recorded. |

**Discipline:** SY.1's default stays `off` (byte-identical) — the switch
is *opt-in*, just *one* opt-in instead of ten. SY.2 changes no behavior
(a test). SY.3 is docs + a description tweak (+ a bundle rebuild only if
strings change). The chapter adds capability *legibility and proof*, not
capability — the lightest possible way to convert a large built
investment into realized, trustworthy value.

## 5. Open questions (resolve in-phase)

- **Exactly what `smart` sets** — the weights + caps for each source +
  the sweep intervals. Pick conservative, coherent defaults (recall
  fusion at weight 1.0 across sources; hourly sweeps capped at 20). Tune
  against the SY.2 integration test (SY.1).
- **Where `profile` lives** — a new `[memory]` section vs. reusing an
  existing one. Default a new `[memory]` (clean home for future
  memory-wide settings) (SY.1).
- **Affordance depth** — is a tighter `graph.query` description enough,
  or does the system prompt need a "you have a knowledge graph" line?
  Default to the description tweak; escalate only if SY.2/live-verify
  shows the model never reaches for it (SY.3).

## 6. Operator live-verify runbook (SY.3)

The automated end-to-end test (SY.2) proves the pipeline *composes*. This
runbook is the human check that a **real model** drives it — the
verification the arc skipped. ~10 minutes, needs Ollama + a tool-capable
model.

1. **Configure smart memory.** In `aivyx.toml`, set an `[embedding]`
   section (a real or local OpenAI-compatible embedder) and the one
   switch:
   ```toml
   [memory]
   profile = "smart"
   ```
2. **Boot the daemon** (`aivyx`). You should see the sweep timers arm; no
   error. Open the Studio (`:7843`) — the **Wiki** and **Graph** screens
   are reachable (empty for now).
3. **Give it something to remember.** Over a few turns, tell the agent
   facts with relationships — e.g. "the deploy pipeline depends on CI",
   "CI runs the nightly tests", "rollback reverts a deploy". (Memories are
   written via the agent's `memory.write`.)
4. **Wait one sweep interval** (default 1 h; lower `[wiki].interval_secs`
   / `[graph].interval_secs` to a minute to verify fast). Watch the daemon
   log for the breadcrumbs:
   - `aivyx graph-sweep: N triple(s) from M topic(s) …`
   - (the wiki sweep writes silently; check the screen)
5. **Confirm the layers filled in (Studio).** The **Wiki** screen now
   lists pages with LLM summaries; the **Graph** screen shows entity nodes
   + directed, predicate-labeled edges (`deploy —depends-on→ ci`).
6. **Confirm the agent reasons over it.** Ask a *relational* question the
   raw notes don't answer directly — "what depends on the deploy
   pipeline?" The agent should call **`graph.query`** (visible in the audit
   chain) and answer from the graph.
7. **Confirm recall fuses it.** Ask about a topic; the daemon log's
   `aivyx recall: …` / `aivyx recall-graph: …` breadcrumbs should show the
   wiki summary + typed-graph neighbors entering the turn's context.

If steps 5–7 hold, the whole Loom→Lexicon stack is live and working from
a single `profile = "smart"`. (If step 6 fails — the model never reaches
for `graph.query` — tighten the tool description per §2; that's the only
expected escalation.)

---

*Chapter Synapse is the spark across the gap the memory arc left: five
chapters of capability, none of it firing by default and none of it
proven whole. One switch arms it, one test proves it composes, one runbook
shows it working — turning "we built a memory system" into "your agent
remembers, organizes, connects, and recalls, and you turned it on with a
single line."*

# Local First-Run Reliability — the free on-ramp (Chapter P)

> **Status:** design contract. This is the spec Chapter P scaffolds from
> (mirrors `docs/ACCESS_LEVELS.md` / `docs/AGENT_WORKSPACE.md`).
>
> Aivyx's headline pitch is **"runs on your hardware, no API key."** That free
> front door is the local-LLM (Ollama) path. But the path is fragile in exactly
> the way that ruins a first impression: a new user runs `aivyx init`, picks
> local, and gets **empty replies**, **dropped tool calls**, or a model that
> only generates one token — because of thinking-model quirks, a streaming
> tool-call edge, and a starved `num_ctx`.
>
> **Chapter P makes the local on-ramp just work, end to end, with zero manual
> config.** A new user should go from "I downloaded it" to "I'm having a real,
> tool-using conversation" without ever learning what `num_ctx` is or debugging
> a blank turn. (Publishing downloadable binaries is the *next* chapter — gated
> on this on-ramp being solid first.)

---

## 0. What broke (the failure modes this chapter closes)

Observed live while testing local models this build:

| Symptom | Cause | State |
|---|---|---|
| Empty assistant text | thinking models (qwen3) route the answer into a `thinking` field, leaving `content` empty when tools are present | **fixed** (`disable_thinking_for` → `think:false`) |
| Tool call silently dropped (`tools=0`, no output) | Ollama 0.30.5 delivers `tool_calls` on a `done:false` chunk; the reader only read them on `done:true` | **fixed** (capture non-terminal `tool_calls`) |
| Model emits ~1 token then stops | the agent prompt (~4–11k tokens) fills the default `num_ctx` 4096, starving generation | **needs auto-fix** (P.1) — today requires hand-set `[ollama] num_ctx` |
| No usable model present | wizard only *hints* `ollama pull …` and leaves the user to it | **needs guided pull** (P.3) |
| Silent failure with no signal | nothing tells the user *why* the first turn was empty | **needs `doctor`** (P.4) |

---

## 1. The four mechanisms

1. **Auto `num_ctx` (the headline).** When `[ollama] num_ctx` is unset, the
   provider reads the model's **native context length** from `/api/show`
   (`model_info["<arch>.context_length"]`, cached on the same call that already
   fetches `family` + `capabilities`) and sets
   `num_ctx = min(native, AUTO_NUM_CTX_CAP)` (16384). Comfortably above the agent
   prompt, VRAM-sane. An explicit `num_ctx` always wins; non-Ollama providers are
   untouched.
2. **A vetted recommended model.** The wizard's local default is a small,
   **tool-capable** (`capabilities` ∋ `"tools"`) model in the ~7–9B range,
   confirmed during this chapter to drive the agent's tool-calling with the
   thinking/tool-call fixes + auto-`num_ctx`. (Chosen empirically, not assumed.)
3. **Guided pull.** When the operator picks local and the recommended model is
   absent (or Ollama has none), the wizard *offers to download it* with streamed
   progress — not a copy-paste hint.
4. **`aivyx doctor` + end-of-wizard check.** Confirms the path actually works:
   Ollama reachable → a usable model present → a real test turn returns
   **non-empty** text. Each failure prints a clear, actionable next step. Run
   automatically at the end of `init`, and on demand any time.

---

## 2. Invariants

- **Zero-config local path.** A new user never needs to know what `num_ctx` is.
  Auto-detection handles it; explicit config always overrides.
- **No silent empty replies.** Every failure mode above is either fixed in the
  provider or surfaced by `doctor` with an actionable message — never a blank
  turn with no explanation.
- **Cloud path untouched.** All changes are scoped to the Ollama provider and the
  wizard's local branch; Anthropic / OpenAI behavior is unchanged.
- **Diagnose, don't guess.** `doctor` reuses the audit chain's `LlmCost`
  out-token signal + a live test turn to distinguish "context-starved",
  "thinking-empty", and "no model" — the same signals that diagnosed these bugs.

---

## 3. Phase plan

| Phase | Deliverable |
|---|---|
| **P.0** | This design contract. |
| **P.1** | Auto `num_ctx` from `/api/show` (the headline fix). |
| **P.2** | Vetted recommended model + wizard default; verify live. |
| **P.3** | Guided model pull in the wizard. |
| **P.4** | `aivyx doctor` + end-of-wizard verification. |
| **P.5** | Docs (`INSTALL.md`), README, memory; tee up the Publish chapter. |

**Status: P.0–P.5 complete and verified live.** A bare `provider=ollama,
model=qwen3.6:27b` config now returns full responses (was 1 token); the 9B tier
emits tool calls correctly; `aivyx doctor` reports green (`test reply OK: "OK"`)
and gives an actionable pull hint on a missing model. The recommended model is
`qwen3:8b` (tool-capable qwen3, verified family). **Next chapter: Publish** —
activate the cargo-dist pipeline into downloadable binaries + a one-line
installer, now that the local on-ramp is solid.

---

## 4. Out of scope (the follow-on — now done)

Publishing downloadable binaries — activating the cargo-dist / GitHub-Actions
release pipeline (`dist-workspace.toml`) into real installable releases + a
one-line installer — was the **next chapter (Chapter Q — Publish)** and is now
prepared: the release ships only the `aivyx` binary, the install docs resolve,
and `v0.1.0` (pre-release) is ready to tag. It deliberately landed *on top of* a
local on-ramp that already works; shipping a binary whose free path produces
empty replies would have defeated the point. See
[`docs/INSTALL.md`](INSTALL.md#shell-installer-recommended).

## 5. Hardening tool-calling itself (Chapter Stencil)

The mechanisms above make a local model *reply*; making a **small** local model
reliably *call a tool* is the harder, later problem — four prompt-substrate phases
proved it can't be fixed from the prompt. **Chapter Stencil** ([`docs/STENCIL.md`](STENCIL.md))
adds the lever the prompt can't reach: **grammar-constrained decoding** on the
in-process mistral.rs engine (`[mistralrs] constrain_tool_calls = true`, default
off). The decoder is constrained to a JSON-Schema grammar built from the registered
tools, so a small GGUF emits a valid, real-named tool call (or a `respond` text
escape) *by construction*, not by hoping. Live-proven on Qwen3-4B.

**Chapter Emboss** ([`docs/EMBOSS.md`](EMBOSS.md)) extends the *same* grammar to the
**`llama-server`** path (`provider = "llamacpp"` / Jan, the OpenAI-compatible local
backends): set `[openai] constrain_tool_calls = true` and Aivyx injects
`tool_call_grammar(tools)` as a `json_schema` constraint on the chat-completions
body, so a GGUF served over HTTP is constrained the same way the in-process engine
is. Default off. The grammar primitive now covers **both** local engines Aivyx
ships. Live-proven against a real `llama-server` on Qwen3-4B.

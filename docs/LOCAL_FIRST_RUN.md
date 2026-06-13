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

---

## 4. Out of scope (the follow-on)

Publishing downloadable binaries — activating the wired-but-dormant cargo-dist /
GitHub-Actions release pipeline (`dist-workspace.toml`) into real installable
releases + a one-line installer — is the **next chapter**. It belongs on top of
a local on-ramp that already works; shipping a binary whose free path produces
empty replies would defeat the point.

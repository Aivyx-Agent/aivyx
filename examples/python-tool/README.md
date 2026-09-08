# Python tool process — reference implementation

A minimal Aivyx PA tool process written in Python 3, demonstrating
that the tool process IPC protocol is language-agnostic. Stdlib
only — no `pip install` step.

This is the worked example referenced from
[`docs/TOOL_SDK.md`](../../docs/TOOL_SDK.md), the sibling of
`examples/python-channel/` shipped in Phase 48.

## What it does

Registers one tool: `wordcount`. Takes `{"text": "<string>"}`,
returns `{"words": N, "chars": N, "lines": N}`.

The tool is functionally trivial on purpose — the point of the
example is the **protocol**, not the work the tool does. A real
tool author swaps the body of `wordcount()` for whatever their
domain needs and keeps everything else.

## Files

| File | Role |
|---|---|
| `frame.py` | Length-prefixed JSON framing — same wire format as `examples/python-channel/frame.py`, vendored so the example is independently runnable |
| `tool.py` | Main loop: handshake → register → serve invocations until `ToolShutdown` / EOF |
| `tests/` | Daemon-free conformance scenarios |

## Hook it up to the daemon

Add to your `aivyx.toml`:

```toml
[[tool_process]]
name = "wordcount"
command = "python3"
args = ["/absolute/path/to/aivyx/examples/python-tool/tool.py"]
```

Start the daemon (`aivyx daemon run`). On startup the daemon
spawns the script, performs the handshake, and registers
`wordcount` in the tool registry. From the agent's perspective
it is indistinguishable from a first-party tool.

## What this proves

The tool implements exactly the lifecycle described in
[`TOOL_SDK.md` § 3](../../docs/TOOL_SDK.md):

1. Read `ToolHello` from stdin.
2. Write `ToolRegister` declaring `wordcount` with
   `required_scope: "memory.read"`.
3. Loop:
   - Read `InvokeTool` → compute → write `ToolResult` with
     `verified: "NotApplicable"`.
   - Read `CancelInvocation` → reply with
     `ToolError { code: "cancelled" }`.
   - Read `ToolShutdown` → exit 0.
   - Read unknown variant → skip (graceful forward-compat).

It declares `required_scope: "memory.read"` because the example
needs a scope that's already registered in `aivyx-capability`'s
`KNOWN_BASES` (a real tool would declare its own dedicated
scope once that scope was added to `KNOWN_BASES` — see
`docs/TOOL_SDK.md` § 4).

## Limitations

- **Synchronous I/O.** `sys.stdin.buffer.read` blocks. Sufficient
  for a single-invocation-at-a-time tool; concurrent invocations
  on the same process would need threads or `asyncio`.
- **No streaming output.** The tool returns a single
  `ToolResult` per invocation. Real long-running tools should
  send `ToolEvent` frames mid-invocation to surface progress.
- **No persistent state.** Each invocation is independent. The
  process is long-lived (one process per daemon lifetime), so
  per-process caches are fine if you want them.

## Running the conformance tests

```sh
python3 -m unittest discover examples/python-tool/tests -v
```

No daemon required, no LLM provider needed — they verify the
SDK-level shapes against the tool process directly via
subprocess + pipes.

#!/usr/bin/env python3
"""Aivyx tool process — `wordcount` reference implementation.

A minimal third-party tool process that:

  1. reads `ToolHello` from stdin
  2. writes `ToolRegister` to stdout announcing one tool, `wordcount`
  3. handles `InvokeTool` requests by returning word/char/line counts
  4. honors `ToolShutdown` by exiting cleanly
  5. honors `CancelInvocation` with a prompt `ToolError` reply

Stdlib only — no pip install step. Mirrors the contract documented
in `docs/TOOL_SDK.md`.

Run it manually:

    python3 examples/python-tool/tool.py

…but you almost never do — the Aivyx daemon spawns this process
itself from a `[[tool_process]]` entry in your `aivyx.toml`.
"""

from __future__ import annotations

import sys
from typing import Any

from frame import FrameError, read_frame, write_frame


PROTOCOL_VERSION = "0.1"


# ---------------------------------------------------------------------------
# Tool implementation — pure function, no I/O.
# ---------------------------------------------------------------------------


def wordcount(input: dict[str, Any]) -> dict[str, Any]:
    text = input.get("text", "")
    if not isinstance(text, str):
        raise ValueError("`text` must be a string")
    return {
        "words": len(text.split()),
        "chars": len(text),
        "lines": text.count("\n") + (1 if text and not text.endswith("\n") else 0),
    }


# ---------------------------------------------------------------------------
# Tool descriptors — what we declare to the daemon at handshake.
# ---------------------------------------------------------------------------


DESCRIPTORS = [
    {
        "name": "wordcount",
        "description": "Count words, chars, and lines in a string.",
        "input_schema": {
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text to count.",
                },
            },
            "required": ["text"],
        },
        # `memory.read` is the safest substrate-scope this example
        # can declare without inventing a new base. A real wordcount
        # tool genuinely reads nothing — `memory.read` is the
        # narrowest pre-registered scope that the daemon's default
        # role envelope holds.
        "required_scope": "memory.read",
    },
]


# Map tool name → implementation. Allows multi-tool processes
# (the protocol supports it; this example just registers one).
HANDLERS = {
    "wordcount": wordcount,
}


# ---------------------------------------------------------------------------
# Main loop.
# ---------------------------------------------------------------------------


def main() -> int:
    stdin = sys.stdin.buffer
    stdout = sys.stdout.buffer

    # Handshake: ToolHello -> ToolRegister.
    hello = read_frame(stdin)
    if not hello or hello.get("type") != "ToolHello":
        print(
            f"aivyx wordcount tool: expected ToolHello, got {hello!r}",
            file=sys.stderr,
        )
        return 2

    write_frame(
        stdout,
        {
            "type": "ToolRegister",
            "tool_process_name": "wordcount",
            "tools": DESCRIPTORS,
        },
    )

    # Per-invocation loop.
    while True:
        try:
            frame = read_frame(stdin)
        except FrameError as exc:
            print(f"aivyx wordcount tool: frame error: {exc}", file=sys.stderr)
            return 3
        if frame is None:
            # Clean EOF — daemon closed our stdin.
            return 0

        ftype = frame.get("type")
        if ftype == "ToolShutdown":
            return 0

        if ftype == "CancelInvocation":
            # We don't run long enough to actually cancel; surface a
            # ToolError so the daemon's pending-call slot resolves
            # promptly.
            write_frame(
                stdout,
                {
                    "type": "ToolError",
                    "call_id": frame.get("call_id", ""),
                    "code": "cancelled",
                    "message": "wordcount was cancelled before completing",
                },
            )
            continue

        if ftype != "InvokeTool":
            # Unknown variant — graceful skip per TOOL_SDK.md § 8.
            continue

        call_id = frame.get("call_id", "")
        tool_name = frame.get("tool_name", "")
        input_payload = frame.get("input", {}) or {}

        handler = HANDLERS.get(tool_name)
        if handler is None:
            write_frame(
                stdout,
                {
                    "type": "ToolError",
                    "call_id": call_id,
                    "code": "unknown_tool",
                    "message": f"this process does not implement tool {tool_name!r}",
                },
            )
            continue

        try:
            output = handler(input_payload)
        except Exception as exc:  # noqa: BLE001 — surface any error to the daemon
            write_frame(
                stdout,
                {
                    "type": "ToolError",
                    "call_id": call_id,
                    "code": "invocation_failed",
                    "message": str(exc),
                },
            )
            continue

        write_frame(
            stdout,
            {
                "type": "ToolResult",
                "call_id": call_id,
                # wordcount is a pure read — `NotApplicable` is the
                # right Verification value (see TOOL_SDK.md § 4).
                "verified": "NotApplicable",
                "output": output,
            },
        )


if __name__ == "__main__":
    sys.exit(main())

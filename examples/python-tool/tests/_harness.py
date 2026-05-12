"""Subprocess-based test harness for the wordcount tool process.

The Aivyx daemon would normally spawn `tool.py` and frame I/O on
its stdin/stdout. These tests do exactly that, using
`subprocess.Popen` with pipes — so the binary under test is the
real `tool.py`, byte-identically to what the daemon runs.

No daemon required, no LLM provider needed, no Aivyx installation
beyond what ships in this repo.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

# Make the sibling `frame` module importable when run as a
# discovered test (`python3 -m unittest discover`).
_HERE = Path(__file__).resolve().parent
_PARENT = _HERE.parent
if str(_PARENT) not in sys.path:
    sys.path.insert(0, str(_PARENT))

from frame import read_frame, write_frame, FrameError  # noqa: E402

TOOL_SCRIPT = _PARENT / "tool.py"


def spawn_tool() -> subprocess.Popen:
    """Spawn the wordcount tool as a subprocess. Returns the
    Popen handle. Caller must `.stdin.close()` and `.wait()` to
    tear down."""
    return subprocess.Popen(
        ["python3", str(TOOL_SCRIPT)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        # Inherit env but force unbuffered binary I/O on the
        # child if it cares (the script reads/writes binary
        # buffers directly, so this is belt-and-braces).
        env={**os.environ, "PYTHONUNBUFFERED": "1"},
    )


def shutdown(proc: subprocess.Popen, timeout: float = 2.0) -> int:
    """Send ToolShutdown and wait for the child to exit. Closes
    all pipe handles to satisfy `ResourceWarning` in unittest
    runs. Returns the exit code."""
    try:
        write_frame(proc.stdin, {"type": "ToolShutdown"})
    except (BrokenPipeError, OSError):
        pass
    for handle in (proc.stdin, proc.stdout, proc.stderr):
        if handle is not None:
            try:
                handle.close()
            except OSError:
                pass
    return proc.wait(timeout=timeout)


__all__ = [
    "FrameError",
    "read_frame",
    "shutdown",
    "spawn_tool",
    "TOOL_SCRIPT",
    "write_frame",
]

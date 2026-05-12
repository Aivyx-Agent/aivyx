"""Test harness — a scripted "daemon" backed by a Unix socketpair.

These tests do not spawn the real `aivyx` binary. Instead, they
use `socket.socketpair()` to give the `DaemonClient` one end of
the pair and the test the other, then script daemon responses
frame by frame.

Rationale: the SDK's correctness lives at the *protocol* layer.
We want to verify that the reference client:

- Reads `DaemonReady` before anything else
- Sends well-formed frames in the right order
- Skips unknown variants gracefully (the SDK contract)
- Surfaces a clean error when the daemon misbehaves

Running these tests should work on any machine with Python 3.11+
and no aivyx installation. Third-party adapter authors should be
able to fork this harness, swap in their adapter, and verify
their conformance the same way.
"""

from __future__ import annotations

import os
import socket
import struct
import sys
from pathlib import Path


# Make sibling modules importable when this file is run from
# `python3 -m unittest discover examples/python-channel/tests`.
_HERE = Path(__file__).resolve().parent
_PARENT = _HERE.parent
if str(_PARENT) not in sys.path:
    sys.path.insert(0, str(_PARENT))


# Re-exported so test modules import from one place.
from frame import encode_frame, read_frame  # noqa: E402
from client import DaemonClient  # noqa: E402

__all__ = ["encode_frame", "read_frame", "DaemonClient", "make_pair"]


def make_pair() -> tuple[socket.socket, socket.socket]:
    """Returns (client_side_for_DaemonClient, daemon_side_for_test).

    Both halves are AF_UNIX SOCK_STREAM — exactly what
    `DaemonClient` expects.
    """
    if os.name != "posix":
        raise RuntimeError("These tests require POSIX (AF_UNIX socketpair).")
    return socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)


def write_frame(sock: socket.socket, message: dict) -> None:
    """Test-side helper — send one length-prefixed JSON frame to the
    DaemonClient under test."""
    sock.sendall(encode_frame(message))


def read_test_frame(sock: socket.socket) -> dict:
    """Test-side helper — read one frame the DaemonClient sent."""
    return read_frame(sock)


def write_raw(sock: socket.socket, payload: bytes) -> None:
    """Send arbitrary bytes (for malformed-frame tests)."""
    sock.sendall(payload)


def write_partial_then_finish(sock: socket.socket, message: dict) -> tuple[bytes, bytes]:
    """Split a frame in half — useful for verifying the client
    correctly handles short reads. Returns (head, tail) — caller
    decides when to send each."""
    full = encode_frame(message)
    half = len(full) // 2
    return full[:half], full[half:]


def encoded_length(message: dict) -> int:
    """How many wire bytes `message` will produce."""
    return 4 + len(encode_frame(message)) - 4

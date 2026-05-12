"""Length-prefixed JSON framing for the Aivyx tool process protocol.

Wire format (per docs/DAEMON_IPC.md):

    +-----------------+-----------------------------------+
    | 4 bytes         | N bytes                           |
    | big-endian u32  | UTF-8 JSON payload                |
    +-----------------+-----------------------------------+

This is a copy of `examples/python-channel/frame.py` — same wire
format, different transport (stdin/stdout instead of Unix socket).
Vendored rather than shared so each example stays independently
runnable.

Stdlib only.
"""

from __future__ import annotations

import io
import json
import struct
import sys
from typing import Any, BinaryIO

MAX_PAYLOAD_SIZE = 16 * 1024 * 1024
FRAME_HEADER_LEN = 4


class FrameError(Exception):
    """Malformed frame on the wire — bail out."""


def encode_frame(message: dict[str, Any]) -> bytes:
    body = json.dumps(message, separators=(",", ":")).encode("utf-8")
    if len(body) > MAX_PAYLOAD_SIZE:
        raise FrameError(f"payload {len(body)} bytes exceeds 16 MiB cap")
    header = struct.pack(">I", len(body))
    return header + body


def write_frame(stream: BinaryIO, message: dict[str, Any]) -> None:
    """Encode `message` and flush it to a binary stream."""
    stream.write(encode_frame(message))
    stream.flush()


def read_frame(stream: BinaryIO) -> dict[str, Any] | None:
    """Block until one full frame is available, decode it, return
    the parsed dict. Returns None on clean EOF (no header bytes
    were read before the stream closed).
    """
    header = _recv_exact(stream, FRAME_HEADER_LEN, allow_eof=True)
    if header is None:
        return None
    (length,) = struct.unpack(">I", header)
    if length > MAX_PAYLOAD_SIZE:
        raise FrameError(f"frame length {length} exceeds 16 MiB cap")
    body = _recv_exact(stream, length, allow_eof=False)
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise FrameError(f"invalid frame body: {exc}") from exc


def _recv_exact(stream: BinaryIO, n: int, allow_eof: bool) -> bytes | None:
    chunks: list[bytes] = []
    remaining = n
    while remaining > 0:
        chunk = stream.read(remaining)
        if not chunk:
            if allow_eof and not chunks:
                return None
            raise FrameError(
                f"EOF in frame after {n - remaining} of {n} bytes"
            )
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)

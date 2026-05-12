"""Length-prefixed JSON framing for the Aivyx daemon IPC protocol.

Wire format (per docs/DAEMON_IPC.md):

    +-----------------+-----------------------------------+
    | 4 bytes         | N bytes                           |
    | big-endian u32  | UTF-8 JSON payload                |
    +-----------------+-----------------------------------+

No trailer, no compression, no TLS. The socket is local-only and
OS-permission-protected.

This module is stdlib-only on purpose — depending on aiohttp /
trio / msgpack / etc. would weaken the "anyone can write an
adapter in any language" point.
"""

from __future__ import annotations

import json
import socket
import struct
from typing import Any

# Per docs/DAEMON_IPC.md.
MAX_PAYLOAD_SIZE = 16 * 1024 * 1024
FRAME_HEADER_LEN = 4


class FrameError(Exception):
    """The connection produced an invalid frame; bail out."""


def encode_frame(message: dict[str, Any]) -> bytes:
    """Serialize `message` to a length-prefixed UTF-8 JSON frame."""
    body = json.dumps(message, separators=(",", ":")).encode("utf-8")
    if len(body) > MAX_PAYLOAD_SIZE:
        raise FrameError(f"payload {len(body)} bytes exceeds 16 MiB cap")
    header = struct.pack(">I", len(body))
    return header + body


def read_frame(sock: socket.socket) -> dict[str, Any]:
    """Block until one full frame is available, then decode it.

    Raises `FrameError` on malformed input or `ConnectionResetError`
    if the peer closed mid-frame.
    """
    header = _recv_exact(sock, FRAME_HEADER_LEN)
    (length,) = struct.unpack(">I", header)
    if length > MAX_PAYLOAD_SIZE:
        raise FrameError(f"frame length {length} exceeds 16 MiB cap")
    body = _recv_exact(sock, length)
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise FrameError(f"invalid frame body: {exc}") from exc


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    """Read exactly n bytes from sock, raising on short reads."""
    chunks: list[bytes] = []
    remaining = n
    while remaining > 0:
        chunk = sock.recv(remaining)
        if not chunk:
            raise ConnectionResetError(
                f"daemon closed connection after {n - remaining} of {n} bytes"
            )
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)

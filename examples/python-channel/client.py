"""High-level Aivyx daemon IPC client.

Wraps the framing layer in `frame.py` with the lifecycle dance
documented in `docs/CHANNEL_SDK.md`:

    connect -> read DaemonReady -> (negotiate) -> StartSession ->
    per-turn (SubmitInput -> events -> TurnComplete) -> Disconnect

Stdlib-only. Synchronous on purpose — a third-party adapter author
should be able to read this end-to-end and understand exactly what
is on the wire.
"""

from __future__ import annotations

import os
import socket
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator

from frame import encode_frame, read_frame


PROTOCOL_VERSION = "0.1"


@dataclass
class TurnEvent:
    """One frame received during a turn."""

    kind: str
    payload: dict[str, Any]


class DaemonClient:
    """Synchronous IPC client. One connection, one session.

    Typical use:

        with DaemonClient.connect() as client:
            client.start_session(role=None, frontend_type="Local")
            for event in client.submit_turn("hello"):
                print(event)
    """

    def __init__(self, sock: socket.socket) -> None:
        self._sock = sock
        self.session_id: str | None = None
        self.daemon_version: str | None = None

    # ---- construction --------------------------------------------------

    @classmethod
    def connect(cls, socket_path: str | os.PathLike[str] | None = None) -> "DaemonClient":
        """Open the Unix socket and consume the DaemonReady frame.

        Falls back to the standard XDG runtime path if `socket_path`
        is None, matching the daemon's `default_socket_path()`.
        """
        if socket_path is None:
            socket_path = _default_socket_path()
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.connect(str(socket_path))
        client = cls(sock)
        client._consume_daemon_ready()
        return client

    # ---- context manager -----------------------------------------------

    def __enter__(self) -> "DaemonClient":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    # ---- handshake -----------------------------------------------------

    def negotiate_protocol(self, version: str = PROTOCOL_VERSION) -> None:
        """Optional — v0.1 always accepts."""
        self._send({"type": "ProtocolNegotiation", "version": version})
        msg = self._recv()
        if msg.get("type") not in ("ProtocolAccepted", "ProtocolRejected"):
            raise ProtocolError(f"unexpected response to negotiation: {msg!r}")
        if msg["type"] == "ProtocolRejected":
            raise ProtocolError(
                f"daemon rejected protocol {version!r}; "
                f"supported: {msg.get('supported')!r}"
            )

    def start_session(
        self, role: str | None = None, frontend_type: str | None = "Local"
    ) -> str:
        """Send StartSession and capture the assigned session_id.

        `frontend_type` defaults to `"Local"` (Trusted tier). See
        `docs/CHANNEL_SDK.md` § 2 for the tier-table.
        """
        self._send(
            {
                "type": "StartSession",
                "role": role,
                "frontend_type": frontend_type,
            }
        )
        msg = self._recv_skipping_unknown(expected={"SessionStarted", "Error"})
        if msg["type"] == "Error":
            raise ProtocolError(f"StartSession failed: [{msg.get('code')}] {msg.get('message')}")
        self.session_id = msg["session_id"]
        return self.session_id

    # ---- turn loop -----------------------------------------------------

    def submit_turn(self, text: str, mission_id: str | None = None) -> Iterator[TurnEvent]:
        """Send a SubmitInput and yield each event until TurnComplete.

        The generator terminates after yielding the TurnComplete
        event. Unknown variants are yielded as `TurnEvent("Unknown",
        payload)` — render-then-skip is the SDK-recommended posture.
        """
        if self.session_id is None:
            raise ProtocolError("submit_turn called before start_session")
        self._send(
            {
                "type": "SubmitInput",
                "session_id": self.session_id,
                "text": text,
                "mission_id": mission_id,
                "attachments": [],
            }
        )
        while True:
            msg = self._recv()
            t = msg.get("type")
            if t == "StreamEvent":
                yield TurnEvent(kind=msg["event"].get("kind", "Unknown"), payload=msg["event"])
            elif t == "TurnComplete":
                yield TurnEvent(kind="TurnComplete", payload=msg)
                return
            elif t == "Error":
                yield TurnEvent(kind="Error", payload=msg)
                return
            elif t in {"MissionCreated", "MissionStateChanged", "GateResolved"}:
                yield TurnEvent(kind=t, payload=msg)
            else:
                # Future-proofing: unknown DaemonMessage variants are
                # not a crash. Yield as Unknown and keep reading.
                yield TurnEvent(kind="Unknown", payload=msg)

    def cancel(self) -> None:
        """Request mid-turn cancellation. The daemon honors it between
        LLM steps; in-flight tool calls still finish."""
        if self.session_id is None:
            return
        self._send({"type": "CancelTurn", "session_id": self.session_id})

    def resolve_gate(self, mission_id: str, gate_id: str, approved: bool) -> None:
        """Answer an ApprovalGate raised mid-turn."""
        self._send(
            {
                "type": "ResolveGate",
                "mission_id": mission_id,
                "gate_id": gate_id,
                "approved": approved,
            }
        )

    # ---- teardown ------------------------------------------------------

    def close(self) -> None:
        """Polite Disconnect, then close the socket."""
        try:
            self._send({"type": "Disconnect"})
        except (OSError, ConnectionError):
            pass
        try:
            self._sock.close()
        except OSError:
            pass

    # ---- internals -----------------------------------------------------

    def _consume_daemon_ready(self) -> None:
        msg = self._recv()
        if msg.get("type") != "DaemonReady":
            raise ProtocolError(
                f"expected DaemonReady as first frame, got {msg!r}"
            )
        self.daemon_version = msg.get("version")

    def _send(self, message: dict[str, Any]) -> None:
        self._sock.sendall(encode_frame(message))

    def _recv(self) -> dict[str, Any]:
        return read_frame(self._sock)

    def _recv_skipping_unknown(self, expected: set[str]) -> dict[str, Any]:
        """Read frames until one matches `expected`. Useful for
        synchronous request/response patterns (e.g., StartSession)
        when an asynchronous lifecycle event might interleave."""
        while True:
            msg = self._recv()
            if msg.get("type") in expected:
                return msg
            if msg.get("type") in {"ShuttingDown"}:
                raise ProtocolError(f"daemon shutting down: {msg.get('reason')}")
            # Skip unknown — future-compat.
            continue


class ProtocolError(Exception):
    """The daemon returned something unexpected for the current step."""


def _default_socket_path() -> Path:
    """Mirror of the daemon's `default_socket_path()`."""
    xdg = os.environ.get("XDG_RUNTIME_DIR")
    if xdg:
        return Path(xdg) / "aivyx" / "daemon.sock"
    home = os.environ.get("HOME")
    if home:
        return Path(home) / ".local" / "share" / "aivyx" / "daemon.sock"
    raise ProtocolError(
        "neither XDG_RUNTIME_DIR nor HOME is set; "
        "cannot determine daemon socket path"
    )

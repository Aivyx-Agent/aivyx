"""Lifecycle conformance — scripted daemon, real DaemonClient.

Drives the client through every shape of the documented lifecycle:

  connect -> DaemonReady -> StartSession -> SubmitInput ->
  StreamEvent(s) -> TurnComplete -> Disconnect

plus the cancel and approval-gate variants, and the
unknown-variant graceful-skip posture required by CHANNEL_SDK.md.

The "daemon" here is one side of a socketpair scripted by the
test; the DaemonClient sees the other side and has no idea it's
not talking to the real binary.
"""

from __future__ import annotations

import threading
import unittest

from _harness import (  # type: ignore[import-not-found]
    DaemonClient,
    make_pair,
    write_frame,
    read_test_frame,
)
from client import ProtocolError


# Helper: build a DaemonClient against the client side of a pair,
# pre-consuming the DaemonReady frame that the test must have
# already written to its side.
def _start_client(daemon_writes_ready_first: bool = True) -> tuple[DaemonClient, "object"]:
    client_sock, daemon_sock = make_pair()
    if daemon_writes_ready_first:
        write_frame(daemon_sock, {"type": "DaemonReady", "version": "0.1"})
    client = DaemonClient(client_sock)
    if daemon_writes_ready_first:
        client._consume_daemon_ready()  # type: ignore[attr-defined]
    return client, daemon_sock


class HappyPath(unittest.TestCase):
    def test_daemon_ready_is_consumed_first(self) -> None:
        client, daemon = _start_client()
        try:
            self.assertEqual(client.daemon_version, "0.1")
        finally:
            daemon.close()
            client.close()

    def test_start_session_returns_session_id(self) -> None:
        client, daemon = _start_client()
        try:
            # In a background thread, the test daemon reads
            # StartSession and replies with SessionStarted.
            def script_daemon() -> None:
                msg = read_test_frame(daemon)
                assert msg["type"] == "StartSession"
                assert msg.get("frontend_type") == "Local"
                write_frame(daemon, {"type": "SessionStarted", "session_id": "test-session-001"})
            t = threading.Thread(target=script_daemon)
            t.start()
            try:
                session_id = client.start_session(role=None, frontend_type="Local")
                self.assertEqual(session_id, "test-session-001")
                self.assertEqual(client.session_id, "test-session-001")
            finally:
                t.join()
        finally:
            daemon.close()
            client.close()

    def test_submit_turn_yields_events_then_completes(self) -> None:
        client, daemon = _start_client()
        try:
            # Skip the StartSession dance — assign session_id by hand.
            client.session_id = "test-session-002"

            def script_daemon() -> None:
                msg = read_test_frame(daemon)
                assert msg["type"] == "SubmitInput"
                assert msg["text"] == "hello"
                write_frame(daemon, {"type": "StreamEvent",
                                     "session_id": "test-session-002",
                                     "event": {"kind": "Text", "text": "Hi "}})
                write_frame(daemon, {"type": "StreamEvent",
                                     "session_id": "test-session-002",
                                     "event": {"kind": "Text", "text": "there"}})
                write_frame(daemon, {"type": "TurnComplete",
                                     "session_id": "test-session-002",
                                     "outcome": "Completed"})
            t = threading.Thread(target=script_daemon)
            t.start()
            try:
                events = list(client.submit_turn("hello"))
                kinds = [e.kind for e in events]
                self.assertEqual(kinds, ["Text", "Text", "TurnComplete"])
                self.assertEqual(events[0].payload["text"], "Hi ")
                self.assertEqual(events[1].payload["text"], "there")
                self.assertEqual(events[2].payload["outcome"], "Completed")
            finally:
                t.join()
        finally:
            daemon.close()
            client.close()


class Cancellation(unittest.TestCase):
    def test_cancel_writes_cancel_turn_frame(self) -> None:
        client, daemon = _start_client()
        try:
            client.session_id = "cancel-test"
            client.cancel()
            msg = read_test_frame(daemon)
            self.assertEqual(msg["type"], "CancelTurn")
            self.assertEqual(msg["session_id"], "cancel-test")
        finally:
            daemon.close()
            client.close()


class ApprovalGate(unittest.TestCase):
    def test_gate_event_surfaces_to_caller(self) -> None:
        client, daemon = _start_client()
        try:
            client.session_id = "gate-test"

            def script_daemon() -> None:
                read_test_frame(daemon)  # SubmitInput
                write_frame(daemon, {"type": "StreamEvent",
                                     "session_id": "gate-test",
                                     "event": {
                                         "kind": "ApprovalGate",
                                         "mission_id": "m-1",
                                         "gate_id": "g-1",
                                         "reason": "approve this",
                                         "scope": "shell.exec",
                                     }})
                # Consume the operator's ResolveGate response.
                resolve = read_test_frame(daemon)
                assert resolve["type"] == "ResolveGate"
                assert resolve["approved"] is True
                write_frame(daemon, {"type": "TurnComplete",
                                     "session_id": "gate-test",
                                     "outcome": "Completed"})
            t = threading.Thread(target=script_daemon)
            t.start()
            try:
                gen = client.submit_turn("escalate")
                gate_event = next(gen)
                self.assertEqual(gate_event.kind, "ApprovalGate")
                self.assertEqual(gate_event.payload["gate_id"], "g-1")
                # Operator-side: resolve the gate, then drain the generator.
                client.resolve_gate(
                    gate_event.payload["mission_id"],
                    gate_event.payload["gate_id"],
                    approved=True,
                )
                remaining = list(gen)
                self.assertEqual(remaining[-1].kind, "TurnComplete")
            finally:
                t.join()
        finally:
            daemon.close()
            client.close()


class UnknownVariantSkip(unittest.TestCase):
    """CHANNEL_SDK.md § 7 requires adapters to accept unknown
    variants by tag and skip them rather than fail."""

    def test_unknown_daemon_message_yields_unknown_kind(self) -> None:
        client, daemon = _start_client()
        try:
            client.session_id = "unk-test"

            def script_daemon() -> None:
                read_test_frame(daemon)
                write_frame(daemon, {"type": "FutureUnknownVariant", "wat": True})
                write_frame(daemon, {"type": "StreamEvent",
                                     "session_id": "unk-test",
                                     "event": {"kind": "Text", "text": "still alive"}})
                write_frame(daemon, {"type": "TurnComplete",
                                     "session_id": "unk-test",
                                     "outcome": "Completed"})
            t = threading.Thread(target=script_daemon)
            t.start()
            try:
                events = list(client.submit_turn("test"))
                kinds = [e.kind for e in events]
                self.assertIn("Unknown", kinds)
                self.assertIn("Text", kinds)
                self.assertEqual(kinds[-1], "TurnComplete")
            finally:
                t.join()
        finally:
            daemon.close()
            client.close()

    def test_unknown_stream_event_kind_yields_unknown_kind(self) -> None:
        client, daemon = _start_client()
        try:
            client.session_id = "unkev-test"

            def script_daemon() -> None:
                read_test_frame(daemon)
                write_frame(daemon, {"type": "StreamEvent",
                                     "session_id": "unkev-test",
                                     "event": {"kind": "FutureRicherEvent",
                                               "weird_field": 42}})
                write_frame(daemon, {"type": "TurnComplete",
                                     "session_id": "unkev-test",
                                     "outcome": "Completed"})
            t = threading.Thread(target=script_daemon)
            t.start()
            try:
                events = list(client.submit_turn("rich"))
                # The unknown StreamEvent kind comes through with kind=
                # "FutureRicherEvent" (per the JSON tag) — the generator
                # passes the event verbatim. This proves a forward-compat
                # adapter can render/ignore without crashing.
                kinds = [e.kind for e in events]
                self.assertIn("FutureRicherEvent", kinds)
                self.assertEqual(kinds[-1], "TurnComplete")
            finally:
                t.join()
        finally:
            daemon.close()
            client.close()


class ErrorSurface(unittest.TestCase):
    def test_start_session_error_raises(self) -> None:
        client, daemon = _start_client()
        try:
            def script_daemon() -> None:
                read_test_frame(daemon)
                write_frame(daemon, {"type": "Error",
                                     "code": "internal",
                                     "message": "boom"})
            t = threading.Thread(target=script_daemon)
            t.start()
            try:
                with self.assertRaises(ProtocolError) as cm:
                    client.start_session()
                self.assertIn("boom", str(cm.exception))
            finally:
                t.join()
        finally:
            daemon.close()
            client.close()

    def test_missing_daemon_ready_raises(self) -> None:
        client_sock, daemon_sock = make_pair()
        try:
            # Daemon writes a non-Ready frame as its first message.
            write_frame(daemon_sock, {"type": "Error",
                                      "code": "init_failed",
                                      "message": "bad init"})
            client = DaemonClient(client_sock)
            with self.assertRaises(ProtocolError):
                client._consume_daemon_ready()  # type: ignore[attr-defined]
        finally:
            client_sock.close()
            daemon_sock.close()


if __name__ == "__main__":
    unittest.main()

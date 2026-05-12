"""Protocol edge-case conformance.

- CancelInvocation produces a ToolError { code: "cancelled" }.
- Unknown wire variants are skipped gracefully (forward-compat).
- Sequential invocations work without re-handshaking.
"""

from __future__ import annotations

import unittest

from _harness import (  # type: ignore[import-not-found]
    read_frame,
    shutdown,
    spawn_tool,
    write_frame,
)


def _handshake(proc):
    write_frame(proc.stdin, {"type": "ToolHello", "protocol_version": "0.1"})
    read_frame(proc.stdout)


class Cancellation(unittest.TestCase):
    def test_cancel_invocation_responds_with_tool_error(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            write_frame(
                proc.stdin,
                {"type": "CancelInvocation", "call_id": "c-cancel"},
            )
            result = read_frame(proc.stdout)
            self.assertEqual(result["type"], "ToolError")
            self.assertEqual(result["call_id"], "c-cancel")
            self.assertEqual(result["code"], "cancelled")
        finally:
            self.assertEqual(shutdown(proc), 0)


class ForwardCompat(unittest.TestCase):
    def test_unknown_variant_is_skipped(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            # Send an unknown variant — the tool must skip it and
            # process the next valid frame.
            write_frame(proc.stdin, {"type": "FutureUnknownVariant", "wat": True})
            write_frame(
                proc.stdin,
                {
                    "type": "InvokeTool",
                    "call_id": "after-unknown",
                    "tool_name": "wordcount",
                    "input": {"text": "alive"},
                    "turn_id": "t-1",
                },
            )
            result = read_frame(proc.stdout)
            self.assertEqual(result["type"], "ToolResult")
            self.assertEqual(result["call_id"], "after-unknown")
            self.assertEqual(result["output"]["words"], 1)
        finally:
            self.assertEqual(shutdown(proc), 0)


class SequentialInvocations(unittest.TestCase):
    def test_two_invocations_share_one_handshake(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            for i, (text, expected_words) in enumerate([
                ("one two three", 3),
                ("a b c d e", 5),
            ]):
                cid = f"seq-{i}"
                write_frame(
                    proc.stdin,
                    {
                        "type": "InvokeTool",
                        "call_id": cid,
                        "tool_name": "wordcount",
                        "input": {"text": text},
                        "turn_id": "t-1",
                    },
                )
                result = read_frame(proc.stdout)
                self.assertEqual(result["type"], "ToolResult")
                self.assertEqual(result["call_id"], cid)
                self.assertEqual(result["output"]["words"], expected_words)
        finally:
            self.assertEqual(shutdown(proc), 0)


if __name__ == "__main__":
    unittest.main()

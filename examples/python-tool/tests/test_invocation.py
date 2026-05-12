"""Invocation conformance: InvokeTool → ToolResult round trip."""

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
    read_frame(proc.stdout)  # ToolRegister


class InvocationRoundTrip(unittest.TestCase):
    def test_wordcount_counts_words_chars_lines(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            write_frame(
                proc.stdin,
                {
                    "type": "InvokeTool",
                    "call_id": "c-1",
                    "tool_name": "wordcount",
                    "input": {"text": "hello world\nphase 49"},
                    "turn_id": "t-1",
                },
            )
            result = read_frame(proc.stdout)
            self.assertEqual(result["type"], "ToolResult")
            self.assertEqual(result["call_id"], "c-1")
            self.assertEqual(result["verified"], "NotApplicable")
            self.assertEqual(result["output"]["words"], 4)
            self.assertEqual(result["output"]["chars"], len("hello world\nphase 49"))
            self.assertEqual(result["output"]["lines"], 2)
        finally:
            self.assertEqual(shutdown(proc), 0)

    def test_empty_text_returns_zeros(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            write_frame(
                proc.stdin,
                {
                    "type": "InvokeTool",
                    "call_id": "c-empty",
                    "tool_name": "wordcount",
                    "input": {"text": ""},
                    "turn_id": "t-1",
                },
            )
            result = read_frame(proc.stdout)
            self.assertEqual(result["output"], {"words": 0, "chars": 0, "lines": 0})
        finally:
            self.assertEqual(shutdown(proc), 0)

    def test_unknown_tool_returns_tool_error(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            write_frame(
                proc.stdin,
                {
                    "type": "InvokeTool",
                    "call_id": "c-bad",
                    "tool_name": "not_a_real_tool",
                    "input": {},
                    "turn_id": "t-1",
                },
            )
            result = read_frame(proc.stdout)
            self.assertEqual(result["type"], "ToolError")
            self.assertEqual(result["call_id"], "c-bad")
            self.assertEqual(result["code"], "unknown_tool")
        finally:
            self.assertEqual(shutdown(proc), 0)

    def test_invalid_input_returns_tool_error(self) -> None:
        proc = spawn_tool()
        try:
            _handshake(proc)
            write_frame(
                proc.stdin,
                {
                    "type": "InvokeTool",
                    "call_id": "c-bad-input",
                    "tool_name": "wordcount",
                    "input": {"text": 42},  # number, not string
                    "turn_id": "t-1",
                },
            )
            result = read_frame(proc.stdout)
            self.assertEqual(result["type"], "ToolError")
            self.assertEqual(result["call_id"], "c-bad-input")
            self.assertEqual(result["code"], "invocation_failed")
        finally:
            self.assertEqual(shutdown(proc), 0)


if __name__ == "__main__":
    unittest.main()

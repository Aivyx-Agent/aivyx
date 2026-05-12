"""Handshake conformance: ToolHello → ToolRegister shape."""

from __future__ import annotations

import unittest

from _harness import (  # type: ignore[import-not-found]
    read_frame,
    shutdown,
    spawn_tool,
    write_frame,
)


class HandshakeShape(unittest.TestCase):
    def test_tool_register_lists_wordcount(self) -> None:
        proc = spawn_tool()
        try:
            write_frame(proc.stdin, {"type": "ToolHello", "protocol_version": "0.1"})
            register = read_frame(proc.stdout)
            self.assertIsNotNone(register)
            self.assertEqual(register["type"], "ToolRegister")
            self.assertEqual(register["tool_process_name"], "wordcount")
            self.assertEqual(len(register["tools"]), 1)
            tool = register["tools"][0]
            self.assertEqual(tool["name"], "wordcount")
            self.assertEqual(tool["required_scope"], "memory.read")
            # Schema declares `text` as a required string property.
            self.assertEqual(tool["input_schema"]["required"], ["text"])
        finally:
            self.assertEqual(shutdown(proc), 0)

    def test_handshake_clean_exit_on_shutdown(self) -> None:
        proc = spawn_tool()
        try:
            write_frame(proc.stdin, {"type": "ToolHello", "protocol_version": "0.1"})
            read_frame(proc.stdout)  # consume ToolRegister
        finally:
            # ToolShutdown should produce a clean exit code 0.
            self.assertEqual(shutdown(proc), 0)


if __name__ == "__main__":
    unittest.main()

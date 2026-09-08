#!/usr/bin/env python3
"""Minimal MCP server for integration testing.

Reads newline-delimited JSON-RPC 2.0 from stdin, writes responses to stdout.
Implements: initialize, notifications/initialized, tools/list, tools/call.
"""
import json
import os
import sys

# Chapter Conduit (CD.5) — env probe. The daemon passes per-server env
# vars to a stdio child; this lets an integration test prove the value
# actually arrived. The tool is only advertised when the var is present,
# so its mere presence is evidence of delivery (and existing tests that
# expect exactly two tools are unaffected).
ENV_PROBE_VAR = "AIVYX_PA_MCP_ENV_PROBE"

TOOLS = [
    {
        "name": "echo",
        "description": "Echoes the input text back",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        },
    },
    {
        "name": "add",
        "description": "Adds two numbers",
        "inputSchema": {
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"},
            },
            "required": ["a", "b"],
        },
    },
]


def handle(msg):
    method = msg.get("method")
    msg_id = msg.get("id")

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "mock-mcp", "version": "0.1.0"},
                "capabilities": {"tools": {}},
            },
        }

    if method == "notifications/initialized":
        return None  # notification, no response

    if method == "tools/list":
        tools = list(TOOLS)
        if ENV_PROBE_VAR in os.environ:
            tools.append({
                "name": "env_probe",
                "description": f"Returns the value of {ENV_PROBE_VAR} as seen by this child",
                "inputSchema": {"type": "object", "properties": {}},
            })
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"tools": tools},
        }

    if method == "tools/call":
        params = msg.get("params", {})
        name = params.get("name")
        args = params.get("arguments", {})

        if name == "echo":
            text = args.get("text", "")
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": False,
                },
            }
        if name == "env_probe":
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "content": [{"type": "text", "text": os.environ.get(ENV_PROBE_VAR, "")}],
                    "isError": False,
                },
            }
        if name == "add":
            a = args.get("a", 0)
            b = args.get("b", 0)
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "content": [{"type": "text", "text": str(a + b)}],
                    "isError": False,
                },
            }
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "content": [{"type": "text", "text": f"unknown tool: {name}"}],
                "isError": True,
            },
        }

    if method == "shutdown":
        return {"jsonrpc": "2.0", "id": msg_id, "result": None}

    if method == "exit":
        sys.exit(0)

    return {
        "jsonrpc": "2.0",
        "id": msg_id,
        "error": {"code": -32601, "message": f"method not found: {method}"},
    }


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        resp = handle(msg)
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()

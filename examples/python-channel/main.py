#!/usr/bin/env python3
"""Aivyx daemon channel — Python CLI REPL.

A minimal third-party channel adapter that connects to the running
Aivyx daemon over the Unix-socket IPC protocol, reads user input
from stdin, and renders streaming agent output to stdout.

Usage:

    python3 main.py [--socket PATH] [--role NAME]

The adapter declares `FrontendType::Local`, so the daemon places
it at `TrustTier::Trusted` for this session.

See docs/CHANNEL_SDK.md for the contract this implements.
"""

from __future__ import annotations

import argparse
import sys

from client import DaemonClient, ProtocolError


def _format_event(kind: str, payload: dict) -> str | None:
    """Render one streaming event to a human-readable line.

    Returns None for events that should be silently absorbed
    (e.g., transient status updates).
    """
    if kind == "Text":
        return payload.get("text", "")
    if kind == "Status":
        return f"\x1b[2m  ⋯ {payload.get('status', '')}\x1b[0m"
    if kind == "ToolCallStarted":
        tool = payload.get("tool_name", "?")
        return f"\x1b[33m  → {tool}\x1b[0m"
    if kind == "ToolCallFinished":
        tool = payload.get("tool_name", "?")
        summary = payload.get("outcome_summary", "")
        return f"\x1b[32m  ← {tool} {summary}\x1b[0m"
    if kind == "ToolOutput":
        return payload.get("chunk", "")
    if kind == "ApprovalGate":
        return (
            f"\x1b[33m  ⚑ APPROVAL GATE [{payload.get('mission_id')}/"
            f"{payload.get('gate_id')}]: {payload.get('reason')}\x1b[0m"
        )
    return None


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Python channel adapter for the Aivyx daemon."
    )
    parser.add_argument(
        "--socket",
        help="Path to the daemon's Unix socket. "
        "Defaults to $XDG_RUNTIME_DIR/aivyx/daemon.sock.",
    )
    parser.add_argument("--role", help="Role to activate (default: server default).")
    args = parser.parse_args()

    try:
        client = DaemonClient.connect(args.socket)
    except (FileNotFoundError, ConnectionRefusedError) as exc:
        print(f"\x1b[31maivyx-python: could not reach daemon: {exc}\x1b[0m", file=sys.stderr)
        print(
            "Is the daemon running? Try `aivyx daemon run` in another terminal.",
            file=sys.stderr,
        )
        return 1

    print(f"connected (daemon {client.daemon_version})", file=sys.stderr)

    try:
        with client:
            session_id = client.start_session(role=args.role, frontend_type="Local")
            print(f"session: {session_id}", file=sys.stderr)
            print("type your message and press Enter. ctrl-d to quit.", file=sys.stderr)

            while True:
                try:
                    line = input("> ")
                except EOFError:
                    print(file=sys.stderr)
                    break
                line = line.strip()
                if not line:
                    continue
                try:
                    for event in client.submit_turn(line):
                        rendered = _format_event(event.kind, event.payload)
                        if rendered:
                            # Text chunks are concatenated without newlines.
                            if event.kind == "Text":
                                sys.stdout.write(rendered)
                                sys.stdout.flush()
                            else:
                                print(rendered)
                        if event.kind == "TurnComplete":
                            print()  # finish the assistant bubble
                            break
                        if event.kind == "Error":
                            print(
                                f"\x1b[31m  ! {event.payload.get('message')}\x1b[0m"
                            )
                            break
                except KeyboardInterrupt:
                    client.cancel()
                    print("\n(cancelled)", file=sys.stderr)
    except ProtocolError as exc:
        print(f"\x1b[31maivyx-python: protocol error: {exc}\x1b[0m", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print(file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

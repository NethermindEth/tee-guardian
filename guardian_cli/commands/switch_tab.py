#!/usr/bin/env python3
"""
Switch tab command for Guardian TUI.
Sends IPC message to TabManager via Unix socket to switch active tab.

Usage:
    python3 -m guardian_cli.commands.switch_tab <tab_index>
"""

import sys
import socket
import json

from ..utils.tab_manager import get_socket_path


def switch_tab(tab_index: int) -> bool:
    """
    Send switch_tab command to TabManager via Unix socket.

    Args:
        tab_index: Tab number (1-indexed) to switch to

    Returns:
        bool: True if successful, False otherwise
    """
    try:
        # Connect to TabManager's Unix socket
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(1.0)  # 1 second timeout
        sock.connect(get_socket_path())

        # Send switch_tab command
        message = {"action": "switch_tab", "tab_index": tab_index}
        sock.sendall(json.dumps(message).encode())

        # Receive response
        data = sock.recv(1024)
        response = json.loads(data.decode())

        sock.close()

        return response.get("status") == "ok"

    except (socket.error, socket.timeout, json.JSONDecodeError, FileNotFoundError):
        # Fail silently to avoid disrupting Zellij UI
        return False


def main():
    """Main entry point for switch_tab command."""
    if len(sys.argv) != 2:
        sys.exit(1)

    try:
        tab_index = int(sys.argv[1])
    except ValueError:
        sys.exit(1)

    success = switch_tab(tab_index)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

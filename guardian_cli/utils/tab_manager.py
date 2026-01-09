"""
Modular tab management for Guardian CLI with Zellij.
Handles spawning, tracking, and switching between multiple tabs via Unix socket IPC.
Includes Zellij layout and config rendering utilities.
Ephemeral - state lives only during process lifetime.
"""

import subprocess
import threading
import socket
import json
import os
from pathlib import Path
from typing import List


def get_socket_path() -> str:
    """Get the control socket path, namespaced by GUARDIAN_PARALLEL if set."""
    parallel_id = os.environ.get("GUARDIAN_PARALLEL")
    if parallel_id:
        return f"/tmp/guardian-parallel-{parallel_id}-control.sock"
    return "/tmp/guardian-control.sock"


def get_status_file_path() -> str:
    """Get the status file path, namespaced by GUARDIAN_PARALLEL if set."""
    parallel_id = os.environ.get("GUARDIAN_PARALLEL")
    if parallel_id:
        return f"/tmp/guardian-parallel-{parallel_id}-status.txt"
    return "/tmp/guardian-status.txt"


def render_zellij_config() -> str:
    """
    Generate a minimal Zellij config KDL string for Guardian.

    This replaces the need for a static zellij-config.kdl file.
    Keybindings use python -m to call switch_tab command.

    Uses shared_except to ensure keybindings work in all modes (normal, scroll, etc).
    Dynamically generates Ctrl+1 through Ctrl+9 keybindings for tab switching.

    If GUARDIAN_PARALLEL is set, includes an env block to propagate it to all panes.

    Returns:
        KDL config string ready to pipe to `zellij --config /dev/stdin`
    """
    # Generate Ctrl+1 through Ctrl+9 keybindings
    tab_keybinds = []
    for i in range(1, 10):
        tab_keybinds.append(f'''        bind "Ctrl {i}" {{
            Run "python3" "-m" "guardian_cli.commands.switch_tab" "{i}" {{ close_on_exit true; }}
        }}''')

    tab_keybinds_str = "\n".join(tab_keybinds)

    # Propagate GUARDIAN_PARALLEL to all panes if set
    parallel_id = os.environ.get("GUARDIAN_PARALLEL")
    env_block = ""
    if parallel_id:
        env_block = f"""env {{
    GUARDIAN_PARALLEL "{parallel_id}"
}}

"""

    config = f"""{env_block}default_mode "normal"

keybinds {{
    shared_except "locked" {{
{tab_keybinds_str}
        bind "Ctrl s" {{ SwitchToMode "Scroll"; }}
        bind "Ctrl q" {{ Quit; }}
    }}
    scroll {{
        bind "Ctrl s" {{ SwitchToMode "Normal"; }}
        bind "Ctrl c" {{ ScrollToBottom; SwitchToMode "Normal"; }}
    }}
}}

theme "default"
mouse_mode false
pane_frames false
on_force_close "quit"
"""
    return config


def render_tab_layout(command: str, args: List[str]) -> str:
    """
    Generate a Zellij tab layout KDL string with status bar + command pane.

    All Guardian tabs follow the same structure:
    - Top pane (9 lines): Status bar reading the namespaced status file
    - Bottom pane (remaining space): Command output

    Args:
        command: Command to run (e.g., "guardian", "python3")
        args: List of arguments (e.g., ["ssh"] or ["-m", "guardian_cli.commands.run_integration_tests"])

    Returns:
        KDL layout string ready to pipe to `zellij action new-tab --layout /dev/stdin`

    Example:
        >>> render_tab_layout("guardian", ["ssh"])
        'layout {\\n    tab split_direction="horizontal" {...}\\n}'
    """
    # Format args for KDL (all args on one line, space-separated, each quoted)
    args_list = " ".join(f'"{arg}"' for arg in args)
    status_file = get_status_file_path()

    layout = f'''layout {{
    tab split_direction="horizontal" {{
        pane size=9 {{
            command "bash"
            args "-c" "while true; do tput cup 0 0; cat {status_file} 2>/dev/null; sleep 0.1; done"
        }}
        pane focus=true {{
            command "{command}"
            args {args_list}
        }}
    }}
}}'''

    return layout


class TabManager:
    """
    Manages multiple Zellij tabs and tracks active tab.
    Provides Unix socket server for IPC with switch-tab commands.

    Example usage:
        manager = TabManager()
        manager.register_tab("Guardian", tab_index=1, is_main=True)
        manager.register_tab("VM-SSH", tab_index=2, command="guardian", args=["ssh"])
        manager.start_control_server()

        # In your rendering loop:
        active_idx = manager.get_active_tab_index()
        tabs = manager.get_tabs_for_display()
    """

    def __init__(self, socket_path=None):
        self.tabs = []  # List of {name, tab_index, layout, spawned, is_main}
        self.active_idx = 0
        self.socket_path = socket_path or get_socket_path()
        self.server_thread = None
        self.running = False
        self.socket_server = None

    def register_tab(self, name, command=None, args=None, is_main=False, default_mode="scroll"):
        """
        Register a Zellij tab.

        Args:
            name: Display name (e.g. "Guardian", "VM-SSH")
            command: Command to run in tab (e.g. "guardian", "python3")
            args: List of command arguments (e.g. ["ssh"] or ["-m", "guardian_cli.commands.run_integration_tests"])
            is_main: Whether this is the main tab (already exists)
            default_mode: Default Zellij mode for this tab ("normal" or "scroll", default: "scroll")
        """

        # Validate default_mode
        if default_mode not in ["normal", "scroll"]:
            raise ValueError(f"Invalid default_mode: {default_mode}. Must be 'normal' or 'scroll'")

        tab = {
            "name": name,
            "tab_index": len(self.tabs) + 1,
            "command": command,
            "args": args or [],
            "spawned": is_main,  # Main tab already exists
            "is_main": is_main,
            "default_mode": default_mode,
        }
        self.tabs.append(tab)
        if is_main:
            self.active_idx = 0

    def spawn_tab(self, tab_index, focus=True):
        """
        Spawn a Zellij tab dynamically without layout files.

        Args:
            tab_index: Tab number (1-indexed)
            focus: If False, switch back to current tab after spawning (default: True)

        Returns:
            bool: True if spawned successfully, False otherwise
        """
        # Save current tab before spawning (1-indexed)
        current_tab_index = self.active_idx + 1

        # Find tab by index
        tab = None
        for t in self.tabs:
            if t["tab_index"] == tab_index:
                tab = t
                break

        if not tab:
            print(f"Tab {tab_index} not registered")
            return False

        # Don't spawn if already spawned
        if tab["spawned"]:
            print(f"Tab {tab_index} ({tab['name']}) already spawned")
            return True

        # Don't spawn if no command
        if not tab["command"]:
            print(f"Tab {tab_index} ({tab['name']}) has no command")
            return False

        try:
            # Generate layout dynamically
            layout_kdl = render_tab_layout(tab["command"], tab["args"])

            # Spawn new tab by piping layout to stdin
            print(f"Spawning tab {tab_index}: {tab['name']}")
            process = subprocess.Popen(
                ["zellij", "action", "new-tab", "--layout", "/dev/stdin", "--name", tab["name"]],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            stdout, stderr = process.communicate(input=layout_kdl)

            if process.returncode == 0:
                tab["spawned"] = True

                # If focus=False, switch back to original tab
                if not focus:
                    subprocess.run(
                        ["zellij", "action", "go-to-tab", str(current_tab_index)],
                        capture_output=True,
                    )
                    print(f"Tab {tab_index} ({tab['name']}) spawned in background")
                else:
                    # Update active index since we're now on the new tab
                    self.active_idx = tab_index - 1
                    print(f"Tab {tab_index} ({tab['name']}) spawned")

                return True
            else:
                print(f"Failed to spawn tab {tab_index}: {stderr}")
                return False

        except Exception as e:
            print(f"Failed to spawn tab {tab_index}: {e}")
            return False

    def switch_to_tab(self, tab_index):
        """
        Switch to a tab (spawn if needed, then switch).
        Automatically switches to the tab's default_mode.

        Args:
            tab_index: Tab number (1-indexed)

        Returns:
            bool: True if switched successfully, False otherwise
        """
        # Find tab by index
        tab = None
        for t in self.tabs:
            if t["tab_index"] == tab_index:
                tab = t
                break

        if not tab:
            print(f"Tab {tab_index} not registered")
            return False

        # Spawn if needed
        if not tab["spawned"]:
            if not self.spawn_tab(tab_index):
                return False

        # Switch to tab
        try:
            subprocess.run(
                ["zellij", "action", "go-to-tab", str(tab_index)],
                capture_output=True,
                text=True,
                check=True,
            )
            self.active_idx = tab_index - 1  # Update active index (0-indexed)

            # Switch to default mode
            mode = tab.get("default_mode", "scroll")
            subprocess.run(
                ["zellij", "action", "switch-mode", mode],
                capture_output=True,
                text=True,
            )
            # Note: No error checking - mode switch failure shouldn't fail the tab switch

            return True

        except subprocess.CalledProcessError as e:
            print(f"Failed to switch to tab {tab_index}: {e}")
            return False

    def get_active_tab_index(self):
        """Get the currently active tab index (0-indexed for display)."""
        return self.active_idx

    def get_tabs_for_display(self):
        """
        Get tabs formatted for status display.

        Returns:
            List of dicts with {name, spawned}
        """
        return [{"name": tab["name"], "spawned": tab["spawned"]} for tab in self.tabs]

    def start_control_server(self):
        """
        Start Unix socket server for IPC.
        Listens for switch-tab commands from guardian CLI.
        """
        # Clean up old socket
        if os.path.exists(self.socket_path):
            os.unlink(self.socket_path)

        self.running = True
        self.server_thread = threading.Thread(target=self._socket_server_loop, daemon=True)
        self.server_thread.start()

    def stop_control_server(self):
        """Stop the socket server."""
        self.running = False
        if self.socket_server:
            self.socket_server.close()
        if self.server_thread:
            self.server_thread.join(timeout=1.0)
        # Clean up socket file
        if os.path.exists(self.socket_path):
            os.unlink(self.socket_path)

    def _socket_server_loop(self):
        """Background thread that listens for commands on Unix socket."""
        try:
            # Create Unix socket
            self.socket_server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.socket_server.bind(self.socket_path)
            self.socket_server.listen(1)
            self.socket_server.settimeout(1.0)  # 1 second timeout for accept

            while self.running:
                try:
                    conn, _ = self.socket_server.accept()
                    with conn:
                        data = conn.recv(1024)
                        if data:
                            try:
                                message = json.loads(data.decode())
                                self._handle_command(message, conn)
                            except json.JSONDecodeError:
                                response = {"status": "error", "message": "Invalid JSON"}
                                conn.sendall(json.dumps(response).encode())
                except socket.timeout:
                    continue  # Check running flag again

        except Exception as e:
            print(f"Socket server error: {e}")
        finally:
            if self.socket_server:
                self.socket_server.close()

    def _handle_command(self, message, conn):
        """
        Handle command received from socket.

        Args:
            message: Dict with command data
            conn: Socket connection to send response
        """
        action = message.get("action")
        tab_index = message.get("tab_index")

        if action == "switch_tab":
            if tab_index is None:
                response = {"status": "error", "message": "Missing tab_index"}
            else:
                success = self.switch_to_tab(tab_index)
                if success:
                    response = {"status": "ok", "message": f"Switched to tab {tab_index}"}
                else:
                    response = {"status": "error", "message": f"Failed to switch to tab {tab_index}"}
        else:
            response = {"status": "error", "message": f"Unknown action: {action}"}

        conn.sendall(json.dumps(response).encode())

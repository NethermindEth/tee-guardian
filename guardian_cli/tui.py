"""
Guardian TUI utilities for Zellij session management and status rendering.
"""

import os
import socket
import subprocess
import time
from pathlib import Path
from rich.progress import Progress

from .utils.tab_manager import TabManager, get_socket_path, get_status_file_path, render_zellij_config
from .utils.status_renderer import start_background_renderer


def launch_zellij(tab_name: str, python_module: str, module_args: list[str] | None = None):
    """
    Bootstrap a Zellij session with a simple initial layout.

    This is a dumb bootstrap function - it just launches Zellij with a static
    layout. The command running inside (python_module) handles its own TUI setup.

    Args:
        tab_name: Display name for the tab
        python_module: Python module to run
        module_args: Optional list of arguments to pass to the Python module
    """
    socket_path = get_socket_path()
    status_file = get_status_file_path()

    # Check if the communication socket is active
    if os.path.exists(socket_path):
        try:
            test_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            test_sock.settimeout(0.5)
            test_sock.connect(socket_path)
            test_sock.close()
            print("Another Guardian Test Session is active. Please wait for it to complete.")
            return
        except (socket.error, socket.timeout, FileNotFoundError):
            pass  # Socket exists but not active, safe to proceed

    # Clean up old status file
    Path(status_file).unlink(missing_ok=True)

    # Render config and layout dynamically (no TabManager needed - just static KDL)
    config_kdl = render_zellij_config()
    layout_kdl = _render_simple_layout(tab_name, python_module, module_args)

    # Write both config and layout to /tmp (ephemeral, cleaned on reboot)
    config_path = "/tmp/guardian_zellij_config.kdl"
    layout_path = "/tmp/guardian_zellij_layout.kdl"
    Path(config_path).write_text(config_kdl)
    Path(layout_path).write_text(layout_kdl)

    try:
        # Launch Zellij with both files (no stdin piping needed)
        subprocess.run(["zellij", "--config", config_path, "--layout", layout_path])
    finally:
        # Clean up files after Zellij exits
        Path(config_path).unlink(missing_ok=True)
        Path(layout_path).unlink(missing_ok=True)


def _render_simple_layout(tab_name: str, python_module: str, module_args: list[str] | None = None) -> str:
    """
    Render a dead simple Zellij layout with status bar + command pane.

    Args:
        tab_name: Display name for the tab
        python_module: Python module to run (e.g. "guardian_cli.commands.run_unit_tests")
        module_args: Optional list of arguments to pass to the Python module

    Returns:
        KDL layout string
    """
    status_file = get_status_file_path()

    # Build args line for KDL
    args_parts = [f'"-m"', f'"{python_module}"']
    if module_args:
        args_parts.extend(f'"{arg}"' for arg in module_args)
    args_line = " ".join(args_parts)

    layout = f'''layout {{
    tab name="{tab_name}" focus=true split_direction="horizontal" {{
        pane size=9 {{
            command "bash"
            args "-c" "while true; do tput cup 0 0; cat {status_file} 2>/dev/null; sleep 0.1; done"
        }}
        pane focus=true {{
            command "python3"
            args {args_line}
        }}
    }}
}}'''
    return layout


def startup_tui(title, tabs_config, metrics) -> tuple[TabManager, Progress]:
    """
    Initialize TUI components: TabManager, Progress, and background renderer.
    Call this at the start of a command when TUI mode is enabled.

    Args:
        title: Panel title (e.g. "Guardian Unit Tests")
        tabs_config: List of tab dicts with {name, command, args, is_main}
        metrics: TestMetrics instance with to_metrics_list() method

    Returns:
        Tuple of (tab_manager, progress)

    Note:
        The stop_renderer callback is automatically registered with atexit,
        so you don't need to manually call cleanup_tui() in most cases.
    """
    status_file = get_status_file_path()
    socket_path = get_socket_path()

    # Cleanup old state files (use namespaced paths)
    for f in [status_file, socket_path]:
        try:
            Path(f).unlink(missing_ok=True)
        except Exception:
            pass

    # Initialize tab manager
    tab_manager = TabManager()
    for tab in tabs_config:
        tab_manager.register_tab(
            name=tab["name"],
            command=tab.get("command"),
            args=tab.get("args", []),
            is_main=tab.get("is_main", False),
            default_mode=tab.get("default_mode", "scroll"),
        )

    # Start control server for tab switching
    tab_manager.start_control_server()

    # Start background renderer
    progress = start_background_renderer(
        title=title,
        tab_manager=tab_manager,
        metrics_callback=metrics.to_metrics_list,
    )

    # Switch to scroll mode immediately so users can scroll during test execution
    subprocess.run(["zellij", "action", "switch-mode", "scroll"], capture_output=True)

    return tab_manager, progress


def wait_for_user_exit():
    """Wait for user to exit TUI, then clean up Zellij session."""
    # Get session name from environment (only set if running inside Zellij)
    session_name = os.environ.get("ZELLIJ_SESSION_NAME")

    print("\nGuardian Tests Completed — Press Ctrl+Q to exit TUI")
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    finally:
        # Clean up Zellij session if we're running inside one
        if session_name:
            try:
                subprocess.run(["zellij", "delete-session", session_name, "--yes"], capture_output=True, timeout=5)
            except Exception:
                pass  # Ignore cleanup errors

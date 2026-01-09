"""
Shared status rendering for Guardian CLI.
Renders Rich Panel to string for file-based display.
Includes progress utilities and background rendering.

IMPORTANT: This module uses completely isolated Console instances that write
to StringIO buffers, NOT to stdout/stderr. This prevents any interference
with subprocess streams (like cargo test output) and the shared console used
elsewhere in the application.
"""

import atexit
import io
import threading
import time

from rich.console import Console
from rich.layout import Layout
from rich.panel import Panel
from rich.progress import BarColumn, Progress, SpinnerColumn, Task, TextColumn, TimeElapsedColumn
from rich.text import Text

from .tab_manager import TabManager, get_status_file_path
from .vm_monitor import VMMonitor, VMStatus

# Shared constants for test commands
HEALTH_PORT = 8443

# Status icons for VM state display (Rich markup)
VM_STATUS_ICONS = {
    VMStatus.BOOTSTRAPPING: "[yellow]◐[/yellow]",
    VMStatus.LEARNER: "[yellow]◑[/yellow]",
    VMStatus.CANDIDATE: "[cyan]◎[/cyan]",
    VMStatus.FOLLOWER: "[green]●[/green]",
    VMStatus.LEADER: "[green bold]★[/green bold]",
    VMStatus.SSH_ONLINE: "[green]●[/green]",
    VMStatus.INVALID: "[red]✗[/red]",
    VMStatus.OFFLINE: "[yellow]⚠[/yellow]",
    VMStatus.STOPPED: "[dim]○[/dim]",
}

# Node state display strings (Rich markup)
NODE_STATE_DISPLAY = {
    "BOOTSTRAPPING": "[yellow]Bootstrap[/yellow]",
    "LEARNER": "[yellow]Learner  [/yellow]",
    "CANDIDATE": "[cyan]Candidate[/cyan]",
    "FOLLOWER": "[green]Follower [/green]",
    "LEADER": "[green bold]Leader   [/green bold]",
    "INVALID": "[red]Invalid  [/red]",
}


class ClusterVMMetrics:
    """Metrics tracking for cluster tests with VMMonitor.

    Displays VM status in a two-column layout: Registry (first 5) and Gossip (last 5).
    Used by both bootstrap and integration tests.
    """

    def __init__(self, monitor: VMMonitor):
        self.monitor = monitor
        self.phase = "Initializing..."

    def _format_vm_line(self, vm) -> str:
        """Format a single VM status line."""
        icon = VM_STATUS_ICONS.get(vm.status, "[dim]?[/dim]")
        octet = vm.ip.split(".")[-1]
        if vm.node_state is None or vm.term is None or vm.verified_peers is None:
            return f"{icon} [dim].{octet:<3} {'---':<9} T-- -P[/dim]"
        else:
            node_state = NODE_STATE_DISPLAY.get(vm.node_state, f"[white]{vm.node_state:<9}[/white]")
            term_str = f"[yellow]T{vm.term:<2}[/yellow]"
            peers_str = f"[magenta]{vm.verified_peers}P[/magenta]"
            return f"{icon} .{octet:<3} {node_state} {term_str} {peers_str}"

    def to_metrics_list(self):
        """Convert VM states to two-column format: Registry (first 5) and Gossip (last 5)."""
        # Split into registry (first 5) and gossip (last 5)
        registry_vms = self.monitor.vms[:5]
        gossip_vms = self.monitor.vms[5:10] if len(self.monitor.vms) > 5 else []

        lines = []
        for i in range(5):
            left = ""
            right = ""

            if i < len(registry_vms):
                left = self._format_vm_line(registry_vms[i])
            else:
                left = " " * 28

            if i < len(gossip_vms):
                right = self._format_vm_line(gossip_vms[i])
            else:
                right = ""

            lines.append(f"{left} [dim]│[/dim] {right}")

        return lines


class ScrollingProgress(Progress):
    """
    Progress wrapper that auto-hides old tasks when exceeding max_visible.
    Maintains FIFO queue of tasks - oldest tasks scroll off when new ones are added.
    """

    def __init__(self, *args, max_visible=5, **kwargs):
        super().__init__(*args, **kwargs)
        self._task_queue = []  # Ordered list of task IDs (FIFO)
        self._max_visible = max_visible

    def add_task(self, description, start=True, total=100.0, completed=0, visible=True, **fields):
        """Add task and auto-hide oldest if exceeding max_visible."""
        task_id = super().add_task(
            description, start=start, total=total, completed=completed, visible=visible, **fields
        )
        self._task_queue.append(task_id)

        # Hide oldest tasks if we exceed max_visible
        visible_count = sum(1 for tid in self._task_queue if self.tasks[tid].visible)
        while visible_count > self._max_visible:
            # Find oldest visible task and hide it
            for tid in self._task_queue:
                if self.tasks[tid].visible:
                    self.update(tid, visible=False)
                    visible_count -= 1
                    break

        return task_id


class ConciseTimeElapsedColumn(TimeElapsedColumn):
    """Renders time elapsed in a concise format."""

    def render(self, task: Task) -> Text:
        """Show time elapsed in concise format."""
        text = super().render(task)
        time_str = text.plain

        if time_str == "-:--:--":
            return text

        # Split the time string (format: H:MM:SS or H:M:S)
        parts = time_str.split(":")
        if len(parts) == 3:
            hours, minutes, seconds = parts

            # Format concisely: only show hours if > 0
            if int(hours) > 0:
                return Text(f"{hours}h {minutes}m {seconds}s", style="progress.elapsed")
            elif int(minutes) > 0:
                return Text(f"{minutes}m {seconds}s", style="progress.elapsed")
            else:
                return Text(f"{seconds}s", style="progress.elapsed")

        return text


def render_status_panel(title, tabs, active_idx, metrics_content, progress_obj):
    """
    Render status panel to ANSI string.

    Args:
        title: Panel title (e.g. "Guardian Unit Tests")
        tabs: List of tabs dicts with name, spawned (from PaneManager)
        active_idx: Index of active pane
        metrics_content: List of strings (with markup) or single Rich renderable for left area
        progress_obj: Rich Progress object (persistent, stateful)

    Returns:
        ANSI-formatted string ready to write to file
    """
    # Create completely isolated console for rendering (never touches stdout/stderr)
    # This console is ONLY used for capturing rendered output to string
    import io

    isolated_buffer = io.StringIO()
    console = Console(
        file=isolated_buffer,
        force_terminal=True,
        width=None,
        height=6,
        legacy_windows=False,
        force_interactive=False,
    )

    # Create main layout: top row split left/right, bottom row split left/right
    layout = Layout()
    layout.split_column(Layout(name="top", ratio=4), Layout(name="bottom", size=1))

    # TOP ROW: 2/3 left (metrics), 1/3 right (progress)
    layout["top"].split_row(Layout(name="metrics", ratio=2), Layout(name="progress", ratio=1))

    # LEFT TOP: Metrics (from caller)
    if isinstance(metrics_content, list):
        layout["top"]["metrics"].update(Text.from_markup("\n".join(metrics_content)))
    else:
        layout["top"]["metrics"].update(metrics_content)

    # RIGHT TOP: Progress
    layout["top"]["progress"].update(progress_obj)

    # BOTTOM ROW: 2/3 left (tabs), 1/3 right (help)
    layout["bottom"].split_row(Layout(name="tabs", ratio=2), Layout(name="help", ratio=1))

    # LEFT BOTTOM: Pane bar
    tab_parts = []
    for i, tab in enumerate(tabs):
        pane_name = tab.get("name", f"Pane {i + 1}")
        status_indicator = "●" if tab.get("spawned", False) else "○"
        if i == active_idx:
            tab_parts.append(f"[bold cyan]{status_indicator} {pane_name} ({i + 1})[/bold cyan]")
        else:
            tab_parts.append(f"[dim]{status_indicator} {pane_name} ({i + 1})[/dim]")

    layout["bottom"]["tabs"].update(Text.from_markup(" | ".join(tab_parts)))

    # RIGHT BOTTOM: Help text
    layout["bottom"]["help"].update(Text("Ctrl+Q: Exit | Ctrl+1,2,3: Switch Tab | Ctrl+S: Scroll", style="dim"))

    # Wrap entire layout in single panel
    panel = Panel(
        layout, title=f"[bold cyan]{title}[/bold cyan]", title_align="left", border_style="cyan", padding=(0, 1)
    )

    # Render to isolated buffer (never touches stdout)
    console.print(panel)

    # Get rendered output from isolated buffer
    output = isolated_buffer.getvalue().rstrip("\n")
    return output


def start_background_renderer(
    title,
    tab_manager,
    metrics_callback,
    output_file=None,
    interval=0.1,
) -> Progress:
    """
    Start a background thread that continuously renders status to file.

    Args:
        title: Panel title (e.g. "Guardian Unit Tests")
        tab_manager: TabManager instance for tab management
        progress_obj: Rich Progress object to display
        metrics_callback: Callable that returns list of metric strings
        output_file: Path to write rendered status
        interval: Rendering interval in seconds

    Returns:
        Tuple of (stop_callback, thread) where stop_callback() stops the renderer
    """
    # Use namespaced status file path if not explicitly provided
    if output_file is None:
        output_file = get_status_file_path()

    renderer_running = {"value": True}

    progress_buffer = io.StringIO()
    isolated_progress_console = Console(
        file=progress_buffer,
        force_terminal=True,
        force_interactive=False,
        legacy_windows=False,
    )

    # Initialize Progress object with isolated console and scrolling (max 5 visible tasks)
    progress = ScrollingProgress(
        TextColumn("[progress.description]{task.description}"),
        BarColumn(bar_width=None),
        SpinnerColumn(spinner_name="dots"),
        ConciseTimeElapsedColumn(),
        console=isolated_progress_console,
        expand=True,
        max_visible=5,
    )

    def background_renderer():
        """Background thread that renders status to file."""
        while renderer_running["value"]:
            try:
                if tab_manager is None:
                    time.sleep(interval)
                    continue

                # Get tabs and active index from pane manager
                tabs = tab_manager.get_tabs_for_display()
                active_idx = tab_manager.get_active_tab_index()

                # Get metrics content from callback
                metrics_content = metrics_callback()

                output = render_status_panel(
                    title=title,
                    tabs=tabs,
                    active_idx=active_idx,
                    metrics_content=metrics_content,
                    progress_obj=progress,
                )

                with open(output_file, "w") as f:
                    f.write(output)
            except Exception:
                pass  # Ignore errors, keep rendering

            time.sleep(interval)

    # Start renderer thread
    renderer = threading.Thread(target=background_renderer, daemon=True)
    renderer.start()

    # Return stop callback and thread
    def stop_renderer():
        renderer_running["value"] = False

    atexit.register(stop_renderer)

    return progress


# =============================================================================
# Test TUI configuration helpers
# =============================================================================

DOCKER_SERVICES_BASE = ["measurement-registry", "openobserve"]


def get_docker_cmd(workspace: str, cluster_ids: list[int]) -> str:
    """Build docker compose command for the given clusters."""
    services = DOCKER_SERVICES_BASE + [f"registry-cluster-{cid}" for cid in cluster_ids]
    return f"cd {workspace}/tests/docker && docker compose up --build {' '.join(services)}"


def get_tests_tab_conf(workspace: str, cluster_ids: list[int], vm_ips: list[str]) -> list[dict]:
    """
    Generate tab configuration for test TUI.

    Args:
        workspace: Remote workspace path
        cluster_ids: List of cluster IDs for Docker services
        vm_ips: List of VM IP addresses to create log tabs for

    Returns:
        List of tab configuration dicts for TabManager
    """
    docker_cmd = get_docker_cmd(workspace, cluster_ids)

    tabs = [
        {"name": "Guardian", "is_main": True, "default_mode": "scroll"},
        {
            "name": "Host-Docker",
            "command": "guardian",
            "args": ["ssh-host", "-i", docker_cmd],
            "default_mode": "normal",
        },
    ]

    for ip in vm_ips:
        octet = ip.split(".")[-1]
        tabs.append(
            {
                "name": f".{octet}",
                "command": "guardian",
                "args": ["ssh", "-t", octet, "-i", "hl -F -P --tail 1000 /var/log/guardian.log"],
                "default_mode": "scroll",
            }
        )

    return tabs

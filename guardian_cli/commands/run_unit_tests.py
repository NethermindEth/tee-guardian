#!/usr/bin/env python3
"""Unit tests: run cargo test in guest VM with progress tracking."""

import logging
import subprocess
from pathlib import Path

from guardian_cli.utils import load_config, sync_to_guest_vm, remote_execute
from guardian_cli.utils.vm import build_vm_image, launch_vm, setup_trust_authority_config
from guardian_cli.tui import startup_tui, wait_for_user_exit
from guardian_cli.utils.vm_monitor import VMMonitor, VMStatus, VMState
from rich.progress import Progress

logger = logging.getLogger("guardian.unit_tests")

UNIT_TESTS_TAB_CONF = [
    {"name": "Guardian", "is_main": True, "default_mode": "scroll"},
    {"name": "VM-SSH", "command": "guardian", "args": ["ssh"], "default_mode": "normal"},
]


class TestMetrics:
    """Metrics tracking for unit tests with VM monitor."""

    def __init__(self, monitor: VMMonitor | None):
        self.monitor = monitor
        self.tests_passed = 0
        self.tests_failed = 0

    def to_metrics_list(self):
        """Convert state to metrics display format."""
        metrics = []

        if self.monitor and self.monitor.vms:
            vm = self.monitor.vms[0]
            status_display = {
                VMStatus.STOPPED: "[dim]Stopped[/dim]",
                VMStatus.BOOTING: "[yellow]Booting[/yellow]",
                VMStatus.SSH_ONLINE: "[green]SSH Online[/green]",
                VMStatus.OFFLINE: f"[red]Offline[/red]: {vm.error_message or 'unreachable'}",
            }
            status_str = status_display.get(vm.status, f"[dim]{vm.status.value}[/dim]")
            metrics.append(f"VM {status_str}")

        if self.tests_passed > 0 or self.tests_failed > 0:
            metrics.append(f"[green]{self.tests_passed}[/green] passed [red]{self.tests_failed}[/red] failed")

        return metrics


def main(launch_tui=True):
    """Run unit tests in guest VM."""
    logger.info("=" * 40)
    logger.info("Guardian Unit Tests")
    logger.info("=" * 40)

    config = load_config()

    dev_vm = VMState(vm_id="Guest-VM", ip="10.42.1.10")
    monitor = VMMonitor([dev_vm], check_interval=2.0)
    monitor.start()

    if launch_tui:
        _, progress = startup_tui("Guardian Unit Tests", UNIT_TESTS_TAB_CONF, TestMetrics(monitor))
    else:
        progress = Progress(disable=True)

    logger.info("Checking guest VM status...")
    monitor.wait_for_update(dev_vm, timeout=10)

    if dev_vm.status in (VMStatus.STOPPED, VMStatus.UNKNOWN):
        logger.info("Guest VM not ready, building image...")

        build_vm_image(
            config,
            local_path=str(Path(__file__).parent.parent.parent / "nixos-tdx"),
            remote_path="~/nixos-tdx",
            build_cmd="nix build -f guest-vm.nix -L -o guest-vm-result 2>&1",
            rsync_excludes=["logs/", "*.qcow2", "*result*"],
            progress=progress,
        )

        logger.info("Launching guest VM...")
        vm_launch_task = progress.add_task("Launching Guest VM", total=100, completed=0)

        launch_vm(
            config,
            name="guardian-dev-vm-0",
            mac="52:54:00:12:34:0a",
            ip="10.42.1.10",
            cid=10,
            bios_path="~/nixos-tdx/guest-vm-result/OVMF_DEBUG.fd",
            disk_path="~/nixos-tdx/guest-vm-result/boot.img",
            memory="16G",
        )

        logger.info("Waiting for SSH...")
        monitor.wait_for_status(dev_vm, VMStatus.SSH_ONLINE, timeout=120)
        progress.update(vm_launch_task, completed=100)

        if config.get("trustauthority", {}).get("api_key"):
            logger.info("Configuring Trust Authority...")
            setup_trust_authority_config(config)

    elif dev_vm.status == VMStatus.BOOTING:
        logger.info("Guest VM booting, waiting for SSH...")
        monitor.wait_for_status(dev_vm, VMStatus.SSH_ONLINE, timeout=120)
    else:
        logger.info("Guest VM already running")

    logger.info("Syncing cargo files...")
    cargo_sync_task = progress.add_task("Syncing Cargo Files", total=100, completed=0)
    sync_to_guest_vm()
    progress.update(cargo_sync_task, completed=100)

    dev_vm_ip = "10.42.1.10"
    cache_check = remote_execute(
        config,
        "cd /workspace && "
        'direnv status 2>&1 | grep -q "Found RC allowed 0" && '
        "CACHE=$(ls -t .direnv/flake-profile-*.rc 2>/dev/null | head -1) && "
        '[ -n "$CACHE" ] && '
        '[ ! flake.nix -nt "$CACHE" ] && '
        '[ ! flake.lock -nt "$CACHE" ] && '
        "echo ready || echo needs-setup",
        streaming=False,
        target=dev_vm_ip,
    )
    needs_setup = cache_check.stdout.strip() != "ready"

    if needs_setup:
        logger.info("Building dev environment...")
        dev_env_task = progress.add_task("Building Dev Environment", total=100, completed=0)
        remote_execute(
            config,
            "cd /workspace && direnv allow && direnv exec . true",
            streaming=True,
            target=dev_vm_ip,
            allocate_tty=launch_tui,
        )
        progress.update(dev_env_task, completed=100)
        logger.info("Dev environment ready")

    logger.info("Running cargo test...")
    test_task = progress.add_task("Running Tests", total=100, completed=0)
    remote_execute(
        config,
        'cd /workspace && source "$(ls -t .direnv/flake-profile-*.rc | head -1)" >/dev/null 2>&1 && cargo test --lib --bins --tests --color=always',
        streaming=True,
        target=dev_vm_ip,
        allocate_tty=launch_tui,
    )
    progress.update(test_task, completed=100)

    if launch_tui:
        wait_for_user_exit()


if __name__ == "__main__":
    main()

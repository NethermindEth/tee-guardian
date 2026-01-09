"""Guardian CLI utilities."""

import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, overload

import yaml
from rich.console import Console

from .logging_handler import setup_guardian_logging

setup_guardian_logging()

console = Console(log_time=False, log_path=False)
CONFIG_PATH = Path.home() / ".guardian" / "config.yaml"

# Host IDs map to Nebula subnets
HOSTS = {
    "tdx": {"id": 1, "nebula_ip": "10.42.1.1"},
    "sev": {"id": 2, "nebula_ip": "10.42.2.1"},
}

SSH_OPTIONS = [
    "-o",
    "StrictHostKeyChecking=no",
    "-o",
    "UserKnownHostsFile=/dev/null",
    "-o",
    "LogLevel=ERROR",
    "-o",
    "ControlMaster=auto",
    "-o",
    "ControlPath=~/.ssh/guardian-control-%r@%h:%p",
    "-o",
    "ControlPersist=10m",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=3",
]


@dataclass
class SSHResult:
    returncode: int
    stdout: str
    stderr: str
    duration: float


def load_config() -> dict:
    """Load config from ~/.guardian/config.yaml"""
    if not CONFIG_PATH.exists():
        return {}
    try:
        with open(CONFIG_PATH) as f:
            config = yaml.safe_load(f) or {}
    except Exception:
        return {}

    # Apply parallel workspace suffix if running in parallel mode
    parallel_id = os.environ.get("GUARDIAN_PARALLEL")
    if parallel_id:
        for host in config.get("hosts", {}).values():
            if "workspace" in host:
                host["workspace"] = f"{host['workspace']}-parallel-{parallel_id}"
    return config


def save_config(config: dict) -> None:
    """Save config to ~/.guardian/config.yaml"""
    CONFIG_PATH.parent.mkdir(exist_ok=True)
    with open(CONFIG_PATH, "w") as f:
        yaml.dump(config, f, default_flow_style=False, sort_keys=False)


def get_host_config(config: dict, host_type: str) -> dict:
    """Get config for a specific host type (tdx/sev).

    Returns dict with: address, username, ssh_key, workspace
    """
    hosts = config.get("hosts", {})
    if host_type not in hosts:
        raise ValueError(f"Host '{host_type}' not configured. Run: guardian setup --{host_type}")
    return hosts[host_type]


def get_identity(config: dict) -> str:
    """Get developer identity string from config.

    Returns identity string like 'user@hostname'.
    """
    identity = config.get("identity")
    if not identity:
        raise ValueError("Identity not configured. Run: guardian setup")
    return identity


def find_cert_api_host(config: dict) -> tuple[str, dict]:
    """Find the host that has cert_api=true.

    Returns (host_type, host_config) tuple.
    Raises ValueError if no cert API host is configured.
    """
    hosts = config.get("hosts", {})
    for host_type, host_config in hosts.items():
        if host_config.get("cert_api"):
            return host_type, host_config
    raise ValueError("No host with cert_api=true configured. Run: guardian setup")


def parse_target(target: str) -> tuple[str, str]:
    """Parse target like 'tdx.20' or '10.42.1.20' into (host_type, ip).

    Returns:
        (host_type, full_ip) - e.g., ("tdx", "10.42.1.20")
    """
    if "." in target and target[0].isdigit():
        # Raw IP like 10.42.1.20
        parts = target.split(".")
        if len(parts) == 4 and parts[0] == "10" and parts[1] == "42":
            host_id = int(parts[2])
            for name, info in HOSTS.items():
                if info["id"] == host_id:
                    return name, target
        return "unknown", target

    if "." in target:
        # Format: tdx.20 or sev.30
        host_type, octet = target.split(".", 1)
        host_type = host_type.lower()
        if host_type not in HOSTS:
            raise ValueError(f"Unknown host type: {host_type}. Use: tdx, sev")
        return host_type, f"10.42.{HOSTS[host_type]['id']}.{octet}"

    # Just an octet, default to tdx
    return "tdx", f"10.42.1.{target}"


def ensure_host_ssh(config: dict, host_type: str) -> None:
    """Verify SSH works to a host. Exits on failure."""
    import logging

    logger = logging.getLogger("guardian.ssh")

    host_config = get_host_config(config, host_type)
    logger.info(f"Checking SSH to {host_type} host...")

    cmd = [
        "ssh",
        "-i",
        host_config["ssh_key"],
        *SSH_OPTIONS,
        f"{host_config['username']}@{host_config['address']}",
        "echo ok",
    ]

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        if result.returncode != 0 or "ok" not in result.stdout:
            logger.error(f"SSH to {host_type} failed: {result.stderr}")
            sys.exit(1)
        logger.info(f"SSH to {host_type}: OK")
    except Exception as e:
        logger.error(f"SSH to {host_type} failed: {e}")
        sys.exit(1)


def ensure_nebula(host_type: str) -> None:
    """Verify Nebula connectivity to a host type."""
    import socket

    import click

    nebula_ip = HOSTS[host_type]["nebula_ip"]
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(2)
    try:
        sock.connect((nebula_ip, 22))
    except (socket.timeout, ConnectionRefusedError, OSError):
        raise click.ClickException(f"Cannot reach {host_type} via Nebula ({nebula_ip}). Run: guardian nebula join")
    finally:
        sock.close()


def _build_ssh_cmd(config: dict, target: str | None = None, tty: bool = False, host_type: str = "tdx") -> list:
    """Build SSH command for target."""
    host_config = get_host_config(config, host_type)
    cmd = ["ssh", "-i", host_config["ssh_key"]] + SSH_OPTIONS

    if tty:
        cmd.append("-tt")
    else:
        cmd.extend(["-o", "BatchMode=yes"])

    if target:
        # SSH to a VM via its Nebula IP
        cmd.append(f"root@{target}")
    else:
        # SSH to the host itself
        cmd.append(f"{host_config['username']}@{host_config['address']}")

    return cmd


@overload
def remote_execute(
    config: dict,
    command: str,
    streaming: Literal[True],
    timeout: int | None = 300,
    target: str | None = None,
    raise_on_nonzero: bool = True,
    allocate_tty: bool = False,
    host_type: str = "tdx",
) -> int: ...


@overload
def remote_execute(
    config: dict,
    command: str,
    streaming: Literal[False],
    timeout: int | None = 300,
    target: str | None = None,
    raise_on_nonzero: bool = True,
    allocate_tty: bool = False,
    host_type: str = "tdx",
) -> SSHResult: ...


def remote_execute(
    config: dict,
    command: str,
    streaming: bool = True,
    timeout: int | None = 300,
    target: str | None = None,
    raise_on_nonzero: bool = True,
    allocate_tty: bool = False,
    host_type: str = "tdx",
) -> SSHResult | int:
    """Execute command via SSH on a host or VM.

    Args:
        config: Guardian config dict
        command: Command to execute
        streaming: If True, stream output to stdout (returns exit code)
                   If False, capture output (returns SSHResult)
        timeout: Command timeout in seconds (None = no timeout)
        target: VM IP to connect to (None = connect to host)
        raise_on_nonzero: Raise RuntimeError on non-zero exit
        allocate_tty: Allocate pseudo-TTY (for interactive commands)
        host_type: Host type (tdx/sev)
    """
    start = time.time()
    ssh_cmd = _build_ssh_cmd(config, target, tty=allocate_tty, host_type=host_type)
    ssh_cmd.append(command)

    if streaming:
        result = subprocess.run(ssh_cmd, timeout=timeout)
        if raise_on_nonzero and result.returncode != 0:
            raise RuntimeError(f"Command failed with exit code {result.returncode}")
        return result.returncode
    else:
        result = subprocess.run(ssh_cmd, capture_output=True, text=True, timeout=timeout)
        ssh_result = SSHResult(result.returncode, result.stdout, result.stderr, time.time() - start)
        if raise_on_nonzero and ssh_result.returncode != 0:
            raise RuntimeError(f"Command failed: {ssh_result.stderr or ssh_result.stdout}")
        return ssh_result

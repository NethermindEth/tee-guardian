"""Nebula overlay network management."""

import base64
import json
import os
import socket
import subprocess
from pathlib import Path

import click

from .utils import HOSTS, load_config, get_identity, find_cert_api_host, remote_execute

NEBULA_DIR = Path.home() / ".guardian" / "nebula"
PIDFILE = NEBULA_DIR / "nebula.pid"
STATE_FILE = NEBULA_DIR / "state.json"


def _is_running() -> int | None:
    """Check if nebula is running, return PID or None."""
    if not PIDFILE.exists():
        return None
    try:
        pid = int(PIDFILE.read_text().strip())
        os.kill(pid, 0)
        return pid
    except (ValueError, ProcessLookupError, PermissionError):
        PIDFILE.unlink(missing_ok=True)
        return None


def _check_reachable(ip: str, port: int = 22, timeout: float = 2) -> bool:
    """Check if a host is reachable."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect((ip, port))
        return True
    except Exception:
        return False
    finally:
        sock.close()


@click.group()
def nebula():
    """Manage Nebula overlay network."""
    pass


@nebula.command()
def join():
    """Join Nebula network."""
    if _is_running():
        raise click.ClickException("Already connected. Run: guardian nebula leave")

    config = load_config()
    identity = get_identity(config)

    try:
        cert_api_host_type, cert_api_host = find_cert_api_host(config)
    except ValueError as e:
        raise click.ClickException(str(e))

    click.echo(f"Allocating certificate for '{identity}'...")

    try:
        result = remote_execute(
            config,
            f"nebula-user-cert allocate '{identity}'",
            streaming=False,
            host_type=cert_api_host_type,
            raise_on_nonzero=False,
        )
    except Exception as e:
        raise click.ClickException(f"Failed to allocate cert: {e}")

    if result.returncode != 0:
        raise click.ClickException(f"Failed to allocate cert: {result.stderr}")

    try:
        cert = json.loads(result.stdout)
    except json.JSONDecodeError:
        raise click.ClickException(f"Invalid cert response: {result.stdout[:200]}")

    if "error" in cert:
        raise click.ClickException(cert["error"])

    # Write certs
    NEBULA_DIR.mkdir(parents=True, exist_ok=True)
    (NEBULA_DIR / "ca.crt").write_bytes(base64.b64decode(cert["ca"]))
    (NEBULA_DIR / "host.crt").write_bytes(base64.b64decode(cert["cert"]))
    (NEBULA_DIR / "host.key").write_bytes(base64.b64decode(cert["key"]))
    (NEBULA_DIR / "host.key").chmod(0o600)

    # Build lighthouse config
    lighthouse_ip = cert.get("lighthouse_ip", HOSTS.get(cert_api_host_type, {}).get("nebula_ip", "10.42.1.1"))
    lighthouse_addr = cert.get("lighthouse", f"{cert_api_host['address']}:4242")
    static_host_map = {lighthouse_ip: [lighthouse_addr]}
    lighthouse_hosts = [lighthouse_ip]

    for host_type, host_config in config.get("hosts", {}).items():
        nebula_ip = host_config.get("nebula_address", HOSTS.get(host_type, {}).get("nebula_ip"))
        if nebula_ip and nebula_ip != lighthouse_ip:
            static_host_map[nebula_ip] = [f"{host_config['address']}:4242"]
            lighthouse_hosts.append(nebula_ip)

    # Write nebula config
    nebula_config = {
        "pki": {
            "ca": str(NEBULA_DIR / "ca.crt"),
            "cert": str(NEBULA_DIR / "host.crt"),
            "key": str(NEBULA_DIR / "host.key"),
        },
        "static_host_map": static_host_map,
        "lighthouse": {"am_lighthouse": False, "hosts": lighthouse_hosts},
        "listen": {"host": "0.0.0.0", "port": 0},
        "punchy": {"punch": True},
        "tun": {"dev": "nebula-guardian", "mtu": 1300},
        "firewall": {
            "outbound": [{"port": "any", "proto": "any", "host": "any"}],
            "inbound": [{"port": "any", "proto": "any", "host": "any"}],
        },
    }
    (NEBULA_DIR / "config.json").write_text(json.dumps(nebula_config, indent=2))

    # Save state
    STATE_FILE.write_text(json.dumps({"ip": cert["ip"], "identity": identity, "expires": cert.get("expires")}))

    # Start nebula daemon
    cmd = ["nebula", "-config", str(NEBULA_DIR / "config.json")]
    if os.geteuid() != 0:
        cmd = ["sudo"] + cmd

    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    PIDFILE.write_text(str(proc.pid))
    click.echo(f"[OK] Connected ({cert['ip']})")


@nebula.command()
def leave():
    """Disconnect from Nebula network."""
    pid = _is_running()
    if not pid:
        click.echo("Not connected")
        return

    kill_cmd = ["sudo", "kill", str(pid)] if os.geteuid() != 0 else ["kill", str(pid)]
    subprocess.run(kill_cmd, capture_output=True)
    PIDFILE.unlink(missing_ok=True)
    click.echo("[OK] Disconnected")


@nebula.command()
def status():
    """Check Nebula connection status."""
    if not _is_running():
        raise click.ClickException("Not connected. Run: guardian nebula join")

    state = json.loads(STATE_FILE.read_text()) if STATE_FILE.exists() else {}
    config = load_config()

    click.echo(f"Connected as {state.get('ip', '?')} ({state.get('identity', '?')})")

    # Check host reachability
    click.echo("\nHosts:")
    for host_type, host_config in config.get("hosts", {}).items():
        nebula_ip = host_config.get("nebula_address", HOSTS.get(host_type, {}).get("nebula_ip"))
        if nebula_ip:
            status = "reachable" if _check_reachable(nebula_ip) else "unreachable"
            click.echo(f"  {host_type} ({nebula_ip}): {status}")

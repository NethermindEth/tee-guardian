"""Guardian setup wizard - fully interactive."""

import getpass
import platform
import subprocess
from pathlib import Path

import click

from .utils import load_config, save_config


def _get_ssh_agent_keys() -> list[dict]:
    """Get SSH keys from ssh-agent via 'ssh-add -l'.

    Returns list of dicts with: fingerprint, type, comment
    """
    try:
        result = subprocess.run(["ssh-add", "-l"], capture_output=True, text=True, timeout=5)
        if result.returncode != 0:
            return []

        keys = []
        for line in result.stdout.strip().split("\n"):
            if not line:
                continue
            # Format: "256 SHA256:abc123... comment (ED25519)"
            parts = line.split()
            if len(parts) >= 4:
                keys.append(
                    {
                        "bits": parts[0],
                        "fingerprint": parts[1],
                        "comment": " ".join(parts[2:-1]),
                        "type": parts[-1].strip("()"),
                    }
                )
        return keys
    except Exception:
        return []


def _select_ssh_key(existing_fingerprint: str | None = None) -> tuple[str, str]:
    """Prompt user to select an SSH key from agent.

    Returns (fingerprint, comment) tuple.
    """
    keys = _get_ssh_agent_keys()

    if not keys:
        click.echo("No SSH keys in agent. Add one with: ssh-add ~/.ssh/id_ed25519")
        raise SystemExit(1)

    click.echo("  SSH Key:")
    default_idx = 1
    for i, k in enumerate(keys, 1):
        marker = " (current)" if k["fingerprint"] == existing_fingerprint else ""
        click.echo(f"    {i}. {k['type']} {k['fingerprint'][:20]}... ({k['comment']}){marker}")
        if k["fingerprint"] == existing_fingerprint:
            default_idx = i

    choice = click.prompt("  Select", type=int, default=default_idx)
    if choice < 1 or choice > len(keys):
        raise click.ClickException(f"Invalid choice: {choice}")

    selected = keys[choice - 1]
    return selected["fingerprint"], selected["comment"]


def _test_ssh(host: str, username: str) -> bool:
    """Test SSH connection to a host using agent keys."""
    cmd = [
        "ssh",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "BatchMode=yes",
        f"{username}@{host}",
        "echo ok",
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        return result.returncode == 0 and "ok" in result.stdout
    except Exception:
        return False


def _configure_host(config: dict, host_type: str) -> None:
    """Configure a single host interactively."""
    click.echo(f"\n{host_type.upper()} Host")

    existing = config.get("hosts", {}).get(host_type, {})

    # Address (public IP)
    address = click.prompt(
        "  Address",
        default=existing.get("address", ""),
    )
    if not address:
        raise click.ClickException("Address required")

    # Nebula address
    default_nebula = existing.get("nebula_address", f"10.42.{1 if host_type == 'tdx' else 2}.1")
    nebula_address = click.prompt(
        "  Nebula Address",
        default=default_nebula,
    )

    # Username
    username = click.prompt("  Username", default=existing.get("username", "root"))

    # SSH key selection
    existing_fingerprint = existing.get("ssh_key_fingerprint")
    fingerprint, comment = _select_ssh_key(existing_fingerprint)

    # Test connection
    click.echo("  Testing...", nl=False)
    if _test_ssh(address, username):
        click.echo(" OK")
    else:
        click.echo(" FAILED")
        if not click.confirm("  Continue anyway?", default=False):
            raise SystemExit(1)

    # Workspace
    workspace = click.prompt(
        "  Workspace",
        default=existing.get("workspace", "/root/guardian-workspace"),
    )

    # Certificate API
    cert_api = click.confirm("  Certificate API", default=existing.get("cert_api", False))

    if "hosts" not in config:
        config["hosts"] = {}

    config["hosts"][host_type] = {
        "address": address,
        "nebula_address": nebula_address,
        "username": username,
        "ssh_key_fingerprint": fingerprint,
        "ssh_key_comment": comment,
        "workspace": workspace,
        "cert_api": cert_api,
    }


def run_setup():
    """Run the interactive setup wizard."""
    click.echo("Guardian Setup\n")

    config = load_config()

    # === Identity Section ===
    detected_user = getpass.getuser()
    detected_hostname = platform.node().split(".")[0]
    default_identity = f"{detected_user}@{detected_hostname}"

    existing_identity = config.get("identity", default_identity)
    if existing_identity == default_identity:
        # Just confirm detected identity
        if not click.confirm(f"Identity: {default_identity}", default=True):
            existing_identity = click.prompt("Identity")
    else:
        # Show existing, allow change
        existing_identity = click.prompt("Identity", default=existing_identity)

    config["identity"] = existing_identity

    # === Host Configuration ===
    existing_hosts = list(config.get("hosts", {}).keys())

    # TDX
    has_tdx = "tdx" in existing_hosts
    configure_tdx = click.confirm(
        f"\nConfigure TDX host?",
        default=has_tdx,
    )

    # SEV
    has_sev = "sev" in existing_hosts
    configure_sev = click.confirm(
        "Configure SEV host?",
        default=has_sev,
    )

    if configure_tdx:
        _configure_host(config, "tdx")
    elif "tdx" in config.get("hosts", {}):
        # Remove if user said no and it existed
        if has_tdx and click.confirm("  Remove existing TDX config?", default=False):
            del config["hosts"]["tdx"]

    if configure_sev:
        _configure_host(config, "sev")
    elif "sev" in config.get("hosts", {}):
        if has_sev and click.confirm("  Remove existing SEV config?", default=False):
            del config["hosts"]["sev"]

    # === Trust Authority (Optional) ===
    existing_ta = config.get("trustauthority", {})

    if click.confirm(f"\nConfigure Trust Authority?", default=bool(existing_ta.get("api_key"))):
        api_key = click.prompt(
            "  API Key",
            default=existing_ta.get("api_key", ""),
            hide_input=True,
            show_default=False,
        )

        if api_key:
            region = click.prompt(
                "  Region",
                type=click.Choice(["us", "eu"]),
                default=existing_ta.get("region", "us"),
            )
            config["trustauthority"] = {"api_key": api_key, "region": region}
        else:
            config.pop("trustauthority", None)

    # === Save ===
    save_config(config)

    # === Summary ===
    click.echo(f"\nSaved to ~/.guardian/config.yaml")

    click.echo(f"\nIdentity: {config['identity']}")

    hosts = config.get("hosts", {})
    if hosts:
        click.echo("\nHosts:")
        for ht, hc in hosts.items():
            cert_marker = " [cert-api]" if hc.get("cert_api") else ""
            click.echo(f"  {ht}: {hc['username']}@{hc['address']} ({hc['nebula_address']}){cert_marker}")

    if config.get("trustauthority"):
        click.echo(f"\nTrust Authority: configured ({config['trustauthority']['region']})")

    click.echo("\nNext: guardian nebula join")

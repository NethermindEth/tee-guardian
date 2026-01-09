import os
import click

from .test import test
from .vm_control import vm
from .nebula import nebula


@click.group()
@click.option("--parallel", type=click.IntRange(0, 9), help="Parallel workspace slot (0-9)")
def cli(parallel):
    """Guardian - TEE Development CLI"""
    if parallel is not None:
        os.environ["GUARDIAN_PARALLEL"] = str(parallel)


cli.add_command(test)
cli.add_command(vm)
cli.add_command(nebula)


@cli.command()
def setup():
    """Configure Guardian for TEE development."""
    from .setup import run_setup

    run_setup()


@cli.command()
def config():
    """Show current configuration."""
    from .utils import load_config

    cfg = load_config()

    # Identity
    identity = cfg.get("identity", {})
    if identity:
        click.echo(f"Identity: {identity.get('user', '?')}@{identity.get('hostname', '?')}")
        slot = identity.get("nebula_slot")
        if slot:
            click.echo(f"Nebula IP: 10.42.0.{slot}")
        click.echo()

    # Hosts
    hosts = cfg.get("hosts", {})
    if not hosts:
        click.echo("No hosts configured. Run: guardian setup")
        return

    click.echo("Hosts:")
    for name, h in hosts.items():
        click.echo(f"  {name}: {h.get('username', 'root')}@{h.get('address', '?')}")
        click.echo(f"         workspace: {h.get('workspace', '?')}")

    # Trust Authority
    ta = cfg.get("trustauthority", {})
    if ta.get("api_key"):
        click.echo(f"\nTrust Authority: configured ({ta.get('region', 'us')})")


@cli.command()
@click.argument("target", default="tdx.10")
@click.argument("command", nargs=-1, required=False)
def ssh(target, command):
    """SSH to a VM.

    TARGET format: tdx.20, sev.30, or raw IP like 10.42.1.20

    \b
    Examples:
      guardian ssh tdx.10          # Dev VM on TDX
      guardian ssh sev.10          # Dev VM on SEV
      guardian ssh tdx.20          # Test VM 20 on TDX
      guardian ssh 10.42.1.30      # Raw IP
      guardian ssh tdx.10 "ls -la" # Run command
    """
    from .utils import load_config, remote_execute, parse_target

    config = load_config()
    host_type, ip = parse_target(target)

    if command:
        result = remote_execute(config, " ".join(command), streaming=False, target=ip, host_type=host_type)
        click.echo(result.stdout, nl=False)
        if result.stderr:
            click.echo(result.stderr, nl=False, err=True)
        if result.returncode != 0:
            raise SystemExit(result.returncode)
    else:
        # Interactive shell
        shell = "bash" if target.endswith(".10") or ip.endswith(".10") else "sh"
        exit_code = remote_execute(
            config,
            shell,
            streaming=True,
            target=ip,
            allocate_tty=True,
            raise_on_nonzero=False,
            timeout=None,
            host_type=host_type,
        )
        if exit_code != 0:
            raise SystemExit(exit_code)


@cli.command()
@click.argument("command", nargs=-1, required=False)
@click.option("--host", "-H", type=click.Choice(["tdx", "sev"]), default="tdx", help="Host type")
@click.option("-i", "--inline", "inline_cmd", help="Run inline command (streaming output)")
def ssh_host(command, host, inline_cmd):
    """SSH directly to a TEE host (not a VM).

    \b
    Examples:
      guardian ssh-host                     # TDX host shell
      guardian ssh-host -H sev              # SEV host shell
      guardian ssh-host "docker ps"         # Run command on TDX host
      guardian ssh-host -i "docker compose up" # Run streaming command
    """
    from .utils import load_config, remote_execute

    config = load_config()

    cmd = inline_cmd or (" ".join(command) if command else None)

    if cmd:
        if inline_cmd:
            # Streaming output for inline command
            exit_code = remote_execute(
                config, cmd, streaming=True, allocate_tty=True, raise_on_nonzero=False, timeout=None, host_type=host
            )
            if exit_code != 0:
                raise SystemExit(exit_code)
        else:
            result = remote_execute(config, cmd, streaming=False, host_type=host)
            click.echo(result.stdout, nl=False)
            if result.stderr:
                click.echo(result.stderr, nl=False, err=True)
            if result.returncode != 0:
                raise SystemExit(result.returncode)
    else:
        exit_code = remote_execute(
            config, "bash", streaming=True, allocate_tty=True, raise_on_nonzero=False, timeout=None, host_type=host
        )
        if exit_code != 0:
            raise SystemExit(exit_code)


if __name__ == "__main__":
    cli()

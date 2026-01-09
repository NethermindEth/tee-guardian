"""VM control commands: launch, kill, network partition simulation."""

import click


@click.group()
def vm():
    """Control and manipulate VMs for testing."""
    pass


@vm.command()
@click.option("--target", "-t", required=True, help="Target (e.g., 'tdx.20', 'sev.30', '10.42.1.20')")
@click.option("--bios", "-b", help="BIOS path (default: workspace/test-vm-result/OVMF_DEBUG.fd)")
@click.option("--disk", "-d", help="Disk image path (default: workspace/test-vm-result/boot.img)")
@click.option("--memory", "-m", default="8G", help="Memory allocation (default: 8G)")
@click.option("--cpus", "-c", default=4, type=int, help="Number of CPUs (default: 4)")
def launch(target, bios, disk, memory, cpus):
    """Launch a test VM.

    Examples:
        guardian vm launch --target tdx.20
        guardian vm launch -t sev.30
        guardian vm launch -t 10.42.1.21
        guardian vm launch -t tdx.20 --memory 6G --cpus 3
    """
    from guardian_cli.utils import load_config, get_host_config, parse_target
    from guardian_cli.utils.vm import launch_vm, parse_vm_target

    config = load_config()
    host_type, ip = parse_target(target)
    host_config = get_host_config(config, host_type)
    workspace = host_config["workspace"]

    vm = parse_vm_target(ip.split(".")[-1])  # Get octet from IP

    bios_path = bios or f"{workspace}/test-vm-result/OVMF_DEBUG.fd"
    disk_path = disk or f"{workspace}/test-vm-result/boot.img"

    click.echo(f"Launching {vm.name} on {host_type} (IP: {ip}, MAC: {vm.mac}, CID: {vm.cid})")

    try:
        launch_vm(
            config,
            name=vm.name,
            mac=vm.mac,
            ip=ip,
            cid=vm.cid,
            bios_path=bios_path,
            disk_path=disk_path,
            memory=memory,
            cpus=cpus,
            host_type=host_type,
        )
        click.echo(f"[OK] {vm.name} launched on {host_type}")
    except Exception as e:
        click.echo(f"[FAIL] {vm.name}: {e}", err=True)
        raise SystemExit(1)


@vm.command()
@click.option("--all", "-a", is_flag=True, help="Kill all test VMs")
@click.option("--target", "-t", help="Target (e.g., 'tdx.20', 'sev.30', '10.42.1.20')")
@click.option("--host", "-H", type=click.Choice(["tdx", "sev"]), default="tdx", help="Host for --all")
def kill(all, target, host):
    """Kill VM process(es) (SIGKILL).

    Examples:
        guardian vm kill --all --host tdx
        guardian vm kill --target tdx.20
        guardian vm kill --target 10.42.1.20
    """
    from guardian_cli.utils import load_config, remote_execute, parse_target
    from guardian_cli.utils.vm import parse_vm_target

    config = load_config()

    if all:
        click.echo(f"Killing all test VMs on {host}...")
        remote_execute(
            config, "pkill -9 -f 'guardian-test-vm'", streaming=False, raise_on_nonzero=False, host_type=host
        )
        click.echo(f"[OK] All test VMs killed on {host}")
    elif target:
        host_type, ip = parse_target(target)
        vm = parse_vm_target(ip.split(".")[-1])
        click.echo(f"Killing {vm.name} on {host_type}...")
        remote_execute(config, f"pkill -9 -f '{vm.name}'", streaming=False, raise_on_nonzero=False, host_type=host_type)
        click.echo(f"[OK] {vm.name} killed on {host_type}")
    else:
        click.echo("[FAIL] Must specify --all or --target", err=True)
        raise SystemExit(1)


@vm.command()
@click.option("--target", "-t", required=True, help="Target (e.g., 'tdx.20', '10.42.1.20')")
@click.option("--from", "from_ip", help="Specific peer to disconnect from (optional)")
@click.option(
    "--direction",
    type=click.Choice(["send", "recv", "both"]),
    default="both",
    help="Direction to block: send (outbound), recv (inbound), or both",
)
def disconnect(target, from_ip, direction):
    """Disconnect VM from network (simulate network partition).

    Examples:
        guardian vm disconnect --target tdx.20
        guardian vm disconnect --target tdx.20 --from 10.42.1.21
        guardian vm disconnect --target 10.42.1.20 --direction send
    """
    from guardian_cli.utils import load_config, remote_execute, parse_target

    config = load_config()
    host_type, ip = parse_target(target)

    if from_ip:
        click.echo(f"Disconnecting {ip} from {from_ip} ({direction}) on {host_type}")
    else:
        click.echo(f"Disconnecting {ip} from network ({direction}) on {host_type}")

    rules = []

    if from_ip is None:
        if direction in ["both", "send"]:
            rules.append(f"iptables -I FORWARD -s {ip} -j DROP")
        if direction in ["both", "recv"]:
            rules.append(f"iptables -I FORWARD -d {ip} -j DROP")
    else:
        if direction in ["both", "send"]:
            rules.append(f"iptables -I FORWARD -s {from_ip} -d {ip} -j DROP")
        if direction in ["both", "recv"]:
            rules.append(f"iptables -I FORWARD -s {ip} -d {from_ip} -j DROP")

    for rule in rules:
        remote_execute(config, rule, streaming=False, raise_on_nonzero=False, host_type=host_type)

    click.echo(f"[OK] Applied {len(rules)} iptables rules on {host_type}")


@vm.command()
@click.option("--target", "-t", required=True, help="Target (e.g., 'tdx.20', '10.42.1.20')")
def reconnect(target):
    """Reconnect VM to network (restore connectivity).

    Examples:
        guardian vm reconnect --target tdx.20
        guardian vm reconnect --target 10.42.1.20
    """
    from guardian_cli.utils import load_config, remote_execute, parse_target

    config = load_config()
    host_type, ip = parse_target(target)
    click.echo(f"Reconnecting {ip} on {host_type}...")

    commands = [
        f"iptables -D FORWARD -s {ip} -j DROP 2>/dev/null || true",
        f"iptables -D FORWARD -d {ip} -j DROP 2>/dev/null || true",
    ]

    for cmd in commands:
        remote_execute(config, cmd, streaming=False, raise_on_nonzero=False, host_type=host_type)

    click.echo(f"[OK] VM reconnected on {host_type}")


@vm.command("read-measurements")
@click.option("--host", "-H", type=click.Choice(["tdx", "sev"]), default="tdx", help="Host type")
def read_measurements(host):
    """Read TEE measurements from global cache for current workspace's boot.img.

    Examples:
        guardian vm read-measurements
        guardian vm read-measurements --host sev
        guardian vm read-measurements | grep RTMR
    """
    import sys
    from guardian_cli.utils import load_config, get_host_config, remote_execute

    config = load_config()
    host_config = get_host_config(config, host)
    workspace = host_config["workspace"]
    boot_image = f"{workspace}/test-vm-result/boot.img"
    cache_dir = "/tmp/guardian-rtmr-cache"

    result = remote_execute(
        config,
        f"sha256sum {boot_image} | cut -d' ' -f1",
        streaming=False,
        raise_on_nonzero=True,
        host_type=host,
    )
    boot_hash = result.stdout.strip()

    cache_file = f"{cache_dir}/{boot_hash}/measurements.txt"
    result = remote_execute(
        config,
        f"cat {cache_file}",
        streaming=False,
        raise_on_nonzero=False,
        host_type=host,
    )

    if result.returncode != 0:
        click.echo(f"Measurements not found for boot.img hash {boot_hash[:16]}...", err=True)
        click.echo(f"Cache file: {cache_file}", err=True)
        click.echo(f"Run 'guardian test integration --{host}' to extract measurements.", err=True)
        sys.exit(1)

    click.echo(result.stdout, nl=False)

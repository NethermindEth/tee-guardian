"""VM utilities: QEMU command building, VM launching, and image management."""

import json
import logging
import subprocess
from dataclasses import dataclass
from typing import Optional

from rich.progress import Progress

from . import remote_execute, get_host_config

logger = logging.getLogger("guardian.vm")

BRIDGE_NAME = "br0"

# Nebula overlay network IPs (for developer access)
NEBULA_DEV_VM_IP = "10.42.1.10"
NEBULA_TEST_VM_BASE_IP = "10.42.1.20"

# Bridge network IPs (for host-side operations)
BRIDGE_DEV_VM_IP = "192.168.122.10"
BRIDGE_TEST_VM_BASE_IP = "192.168.122.20"

# Default to Nebula IPs for developer-facing code
DEV_VM_IP = NEBULA_DEV_VM_IP
TEST_VM_BASE_IP = NEBULA_TEST_VM_BASE_IP


@dataclass(slots=True, frozen=True)
class VMTarget:
    """VM target information derived from IP or octet."""

    ip: str
    mac: str
    cid: int
    name: str
    octet: int


def parse_vm_target(target: str | int, host_id: int = 1) -> VMTarget:
    """Parse VM target (IP address or octet) into full VM parameters.

    Args:
        target: Either full IP (e.g., "10.42.1.20") or last octet (e.g., "20" or 20)
        host_id: Nebula host ID (1=TDX, 2=SEV). Default: 1 (TDX)

    Returns:
        VMTarget with ip, mac, cid, name, and octet
    """
    target_str = str(target)

    if "." in target_str:
        octet = int(target_str.split(".")[-1])
        full_ip = target_str
    else:
        octet = int(target_str)
        full_ip = f"10.42.{host_id}.{octet}"

    mac = f"52:54:00:12:34:{octet:02x}"
    cid = octet
    name = f"guardian-test-vm-{octet}"

    return VMTarget(ip=full_ip, mac=mac, cid=cid, name=name, octet=octet)


def build_qemu_command(
    name: str,
    ip: str,
    mac: str,
    cid: int,
    disk: str,
    bios: str,
    tdx: bool = True,
    memory: str = "8G",
    cpus: int = 4,
) -> str:
    """Build QEMU command for bridge-networked VM."""
    if tdx:
        machine = "q35,accel=kvm,kernel-irqchip=split,confidential-guest-support=tdx,hpet=off"
        tdx_obj = '-object \'{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type":"vsock","cid":"2","port":"4050"}}\''
    else:
        machine = "q35,accel=kvm"
        tdx_obj = ""

    parts = [
        "qemu-system-x86_64",
        f"-name {name}",
        f"-machine {machine}",
        "-cpu host",
        f"-smp {cpus}",
        f"-m {memory}",
    ]

    if tdx_obj:
        parts.append(tdx_obj)

    parts.extend(
        [
            f"-bios {bios}",
            f"-drive file={disk},format=raw,if=virtio",
            f"-netdev bridge,id=net0,br={BRIDGE_NAME}",
            f"-device virtio-net-pci,netdev=net0,mac={mac}",
            f"-device vhost-vsock-pci,guest-cid={cid}",
            "-display none",
            f"-serial file:/tmp/{name}-console.log",
            "-daemonize",
        ]
    )

    return " \\\n  ".join(parts)


def setup_trust_authority_config(config):
    """Setup Intel Trust Authority config in guest VM."""
    ta_config = config.get("trustauthority", {})
    if not ta_config or not ta_config.get("api_key"):
        logger.info("No Trust Authority config")
        return

    api_key = ta_config["api_key"]
    region = ta_config.get("region", "us")

    region_urls = {
        "us": {
            "trustauthority_url": "https://portal.trustauthority.intel.com",
            "trustauthority_api_url": "https://api.trustauthority.intel.com",
        },
        "eu": {
            "trustauthority_url": "https://portal.eu.trustauthority.intel.com",
            "trustauthority_api_url": "https://api.eu.trustauthority.intel.com",
        },
    }

    if region not in region_urls:
        logger.warning(f"Invalid Trust Authority region: {region}")
        return

    logger.info(f"Configuring Trust Authority ({region})...")

    config_data = {**region_urls[region], "trustauthority_api_key": api_key}
    config_json = json.dumps(config_data, indent=2)
    target = DEV_VM_IP

    try:
        setup_cmd = f"""
        mkdir -p /etc/trustauthority && \\
        cat > /etc/trustauthority/config.json << 'EOF'
{config_json}
EOF
        chmod 600 /etc/trustauthority/config.json && \\
        chown root:root /etc/trustauthority/config.json
        """
        remote_execute(config, setup_cmd, streaming=False, target=target, timeout=10)
        logger.info("Trust Authority configured")
    except Exception as e:
        logger.warning(f"Trust Authority setup failed: {e}")


def build_vm_image(
    config,
    local_path: str,
    remote_path: str,
    build_cmd: str = "nix build -f guest-vm.nix -L -o guest-vm-result",
    rsync_excludes: Optional[list] = None,
    rsync_includes: Optional[list] = None,
    progress: Optional[Progress] = None,
    silent: bool = False,
    host_type: str = "tdx",
):
    """Build a VM image on a remote host."""
    if rsync_excludes is None:
        rsync_excludes = ["logs/", "*.qcow2", "result", "*-result"]

    host_config = get_host_config(config, host_type)

    if progress:
        sync_task = progress.add_task(f"Syncing to {host_type}", total=100)
    else:
        sync_task = None

    if not silent:
        logger.info(f"Syncing {local_path} -> {host_type}:{remote_path}")

    include_args = " ".join([f"--include='{p}'" for p in rsync_includes]) if rsync_includes else ""
    exclude_args = " ".join([f"--exclude='{p}'" for p in rsync_excludes])

    ssh_opts = f"-i {host_config['ssh_key']} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
    rsync_cmd = f"rsync -azv --delete {include_args} {exclude_args} -e 'ssh {ssh_opts}' {local_path}/ {host_config['username']}@{host_config['address']}:{remote_path}/"

    result = subprocess.run(rsync_cmd, shell=True, text=True)
    if result.returncode != 0:
        raise Exception(f"Failed to sync to {host_type}")

    if progress and sync_task is not None:
        progress.update(sync_task, completed=100)

    if progress:
        build_task = progress.add_task(f"Building on {host_type}", total=100)
        progress.update(build_task, completed=10)
    else:
        build_task = None

    if not silent:
        logger.info(f"Building VM image on {host_type}...")

    remote_execute(
        config, f"cd {remote_path} && {build_cmd}", streaming=True, raise_on_nonzero=True, host_type=host_type
    )

    if progress and build_task is not None:
        progress.update(build_task, completed=100)

    if not silent:
        logger.info("VM image built")


def launch_vm(
    config: dict,
    *,
    name: str,
    mac: str,
    ip: str,
    cid: int,
    bios_path: str,
    disk_path: str,
    memory: str = "8G",
    cpus: int = 4,
    host_type: str = "tdx",
):
    """Launch a VM with the given parameters.

    Args:
        config: Guardian config
        name: VM name
        mac: MAC address
        ip: IP address
        cid: vsock CID
        bios_path: Path to BIOS file
        disk_path: Path to disk image
        memory: Memory allocation (default: "8G")
        cpus: Number of CPUs (default: 4)
        host_type: Host to launch on (tdx/sev)
    """
    check = remote_execute(
        config, f"pgrep -f 'mac={mac}'", streaming=False, raise_on_nonzero=False, host_type=host_type
    )
    if check.returncode == 0:
        raise ValueError(f"MAC address {mac} already in use")

    writable_boot_path = f"/tmp/guardian-{name}.img"
    remote_execute(config, f"rm -f {writable_boot_path}", streaming=False, host_type=host_type)
    logger.info(f"Copying boot image for {name}...")

    remote_execute(
        config,
        f"cp {disk_path} {writable_boot_path} && chmod 644 {writable_boot_path}",
        streaming=False,
        host_type=host_type,
    )

    qemu_cmd = build_qemu_command(name, ip, mac, cid, writable_boot_path, bios_path, memory=memory, cpus=cpus)

    remote_execute(config, qemu_cmd, streaming=False, raise_on_nonzero=True, host_type=host_type)

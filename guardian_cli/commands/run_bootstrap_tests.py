#!/usr/bin/env python3
"""Bootstrap tests runner.

Optimizations:
- Parallel VM image builds across hosts (TDX + SEV simultaneously)
- Early failure detection
"""

import logging
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from guardian_cli.tui import startup_tui, wait_for_user_exit
from guardian_cli.utils import load_config, remote_execute, get_host_config, HOSTS
from guardian_cli.utils.cluster import cluster_id_to_ips, clear_cluster_registry
from guardian_cli.utils.measurements import extract_measurements
from guardian_cli.utils.status_renderer import ClusterVMMetrics, HEALTH_PORT, get_docker_cmd, get_tests_tab_conf
from guardian_cli.utils.vm import build_vm_image
from guardian_cli.utils.vm_monitor import VMMonitor, VMStatus, VMState

logger = logging.getLogger("guardian.bootstrap_tests")


def _kill_vm(target: str) -> None:
    """Kill a single VM by target."""
    subprocess.run(["guardian", "vm", "kill", "-t", target], capture_output=True)


def _build_on_host(config: dict, host_type: str, progress=None) -> tuple[str, bool, str | None]:
    """Build VM image on a single host. Returns (host_type, success, error)."""
    try:
        host_config = get_host_config(config, host_type)
        build_vm_image(
            config,
            local_path=str(Path(__file__).parent.parent.parent),
            remote_path=host_config["workspace"],
            build_cmd="nix build .#hydraJobs.test-vm.all -L -o test-vm-result",
            rsync_includes=[
                "crates/***",
                "releases/***",
                "tests/docker/***",
                "pyproject.toml",
                "uv.lock",
                "Cargo.*",
                "flake.*",
            ],
            rsync_excludes=["*"],
            progress=progress,
            host_type=host_type,
        )
        return host_type, True, None
    except Exception as e:
        return host_type, False, str(e)


def main(
    launch_tui: bool = True,
    test_name: str | None = None,
    cluster_ids: list[int] | None = None,
    host_types: list[str] | None = None,
) -> int:
    """Run bootstrap tests.

    Args:
        launch_tui: Whether to launch the TUI
        test_name: Specific test to run (optional)
        cluster_ids: List of cluster IDs to use
        host_types: List of host types (tdx, sev)

    Returns:
        Exit code (0 = success)
    """
    if not cluster_ids:
        logger.error("cluster_ids required")
        return 1
    if not host_types:
        host_types = ["tdx"]

    logger.info("=" * 60)
    logger.info(f"Guardian Bootstrap Tests")
    logger.info(f"Hosts: {', '.join(host_types)} | Clusters: {cluster_ids}")
    logger.info("=" * 60)

    config = load_config()

    # Get workspace from first configured host
    primary_host = host_types[0]
    primary_config = get_host_config(config, primary_host)
    primary_workspace = primary_config["workspace"]

    # Get all IPs across hosts
    cluster_ips = cluster_id_to_ips(cluster_ids[0], host_types)
    all_ips = []
    for ht in host_types:
        all_ips.extend(cluster_ips[ht])
    all_octets = sorted(set(int(ip.split(".")[-1]) for ip in all_ips))

    # Create VM states for monitoring
    vm_states = [
        VMState(vm_id=f"VM-{ip.split('.')[-1]}", ip=ip, health_url=f"http://{ip}:{HEALTH_PORT}/health")
        for ip in all_ips
    ]

    monitor = VMMonitor(vm_states, check_interval=0.5, health_timeout=5)
    monitor.start()

    metrics = ClusterVMMetrics(monitor)

    if launch_tui:
        tab_conf = get_tests_tab_conf(primary_workspace, cluster_ids, all_ips)
        tab_manager, progress = startup_tui("Guardian Bootstrap Tests", tab_conf, metrics)
    else:
        from rich.progress import Progress

        progress = Progress(disable=True)
        tab_manager = None

    exit_code = 1
    try:
        # === Phase 1: Parallel VM image builds ===
        logger.info(f"Building VM images on {len(host_types)} host(s)...")

        build_failed = False
        if len(host_types) > 1:
            # Parallel builds
            with ThreadPoolExecutor(max_workers=len(host_types)) as executor:
                futures = {executor.submit(_build_on_host, config, ht, progress): ht for ht in host_types}
                for future in as_completed(futures):
                    host_type, success, error = future.result()
                    if success:
                        logger.info(f"[OK] Build completed on {host_type}")
                    else:
                        logger.error(f"[FAIL] Build failed on {host_type}: {error}")
                        build_failed = True
        else:
            # Single host build
            host_type, success, error = _build_on_host(config, host_types[0], progress)
            if not success:
                logger.error(f"[FAIL] Build failed: {error}")
                build_failed = True

        if build_failed:
            logger.error("Build failed, aborting tests")
            return 1

        # === Phase 2: Extract measurements ===
        extraction_octet = all_octets[0]
        try:
            extract_measurements(
                config=config,
                workspace=primary_workspace,
                ip_octet=extraction_octet,
                progress=progress,
                host_type=primary_host,
            )
        except Exception as e:
            logger.error(f"Failed to extract measurements: {e}")
            return 1

        # === Phase 3: Start Docker services ===
        logger.info("Starting Docker services...")
        docker_cmd = get_docker_cmd(primary_workspace, cluster_ids)
        if launch_tui and tab_manager:
            tab_manager.spawn_tab(2, focus=False)
        else:
            remote_execute(config, f"{docker_cmd} -d", streaming=True, host_type=primary_host)

        # === Phase 4: Cleanup existing VMs ===
        if not test_name:
            logger.info("Cleaning up existing VMs...")
            targets = [f"{ht}.{o}" for ht in host_types for o in all_octets]
            with ThreadPoolExecutor(max_workers=len(targets)) as executor:
                list(executor.map(_kill_vm, targets))

            for vm_state in vm_states:
                try:
                    monitor.wait_for_status(vm_state, VMStatus.STOPPED, timeout=10)
                except Exception:
                    pass

        # === Phase 5: Run pytest (bootstrap tests launch their own VMs) ===
        pytest_cmd = [
            "pytest",
            f"--cluster-ids={','.join(map(str, cluster_ids))}",
            f"--host-types={','.join(host_types)}",
            "-v",
            "--tb=short",
            "-m",
            "bootstrap",
        ]

        if test_name:
            if "::" in test_name or test_name.endswith(".py"):
                pytest_cmd.append(f"tests/bootstrap/{test_name}")
            else:
                pytest_cmd.extend(["-k", test_name, "tests/bootstrap"])
        else:
            pytest_cmd.append("tests/bootstrap")

        logger.info(f"Running: {' '.join(pytest_cmd)}")
        result = subprocess.run(pytest_cmd, cwd=str(Path(__file__).parent.parent.parent))
        exit_code = result.returncode

        if exit_code == 0:
            logger.info("[OK] Tests passed")
        else:
            logger.error(f"[FAIL] Tests failed (exit {exit_code})")

        if launch_tui:
            wait_for_user_exit()

    finally:
        monitor.stop()
        for cid in cluster_ids:
            try:
                clear_cluster_registry(config, cid)
            except Exception:
                pass

    return exit_code


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--test", default=None)
    parser.add_argument("--cluster-ids", required=True)
    parser.add_argument("--host-types", default="tdx")
    args = parser.parse_args()

    cids = [int(x.strip()) for x in args.cluster_ids.split(",")]
    htypes = [x.strip() for x in args.host_types.split(",")]
    sys.exit(main(launch_tui=True, test_name=args.test, cluster_ids=cids, host_types=htypes))

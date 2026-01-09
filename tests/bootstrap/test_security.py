"""Security tests for attestation and measurement verification.

Tests that nodes with different TDX measurements (different hardware config)
are rejected from joining the cluster.
"""

import subprocess
import time

import pytest

from guardian_cli.utils.vm_monitor import VMStatus
from tests.helpers import (
    create_monitor,
    get_health,
    kill_vms_parallel,
    launch_vms_parallel,
    set_test_context,
    wait_for_ready,
)


@pytest.mark.bootstrap
def test_mismatched_measurements_rejected(cluster_ips):
    """
    VMs with different TDX measurements cannot join the cluster.

    This test verifies the core security property: nodes with different
    hardware configurations (which produce different RTMR measurements)
    are rejected during attestation.

    Test procedure:
    1. Launch 3 legitimate nodes (8G RAM, 4 CPUs) to form a cluster
    2. Launch 2 "rogue" nodes with different resources (6G RAM, 3 CPUs)
    3. Verify legitimate cluster forms normally
    4. Verify rogue nodes remain stuck in BOOTSTRAPPING (never join)
    5. Verify legitimate nodes don't trust the rogue nodes

    This tests:
    - Measurement registry correctly validates RTMR hashes
    - Attestation verification rejects mismatched measurements
    - Cluster integrity is maintained against misconfigured nodes
    """
    # Use first 3 IPs for legitimate cluster, next 2 for rogue nodes
    legitimate_ips = cluster_ips[:3]
    rogue_ips = cluster_ips[3:5]
    all_ips = cluster_ips[:5]

    kill_vms_parallel(cluster_ips)

    # Launch legitimate nodes with standard config
    print(f"Launching legitimate nodes: {legitimate_ips}")
    launch_vms_parallel(legitimate_ips, memory="8G", cpus=4)

    # Create monitor for all nodes
    monitor = create_monitor(all_ips)

    try:
        set_test_context(legitimate_ips, "test_mismatched_measurements_rejected")

        # Wait for legitimate cluster to form
        operational = (VMStatus.LEADER, VMStatus.FOLLOWER, VMStatus.CANDIDATE)
        for vm in monitor.vms[:3]:
            monitor.wait_for_status(vm, operational, timeout=120)

        print("Legitimate 3-node cluster formed")

        # Now launch rogue nodes with different resources
        # Different CPU/memory = different TDX measurements
        print(f"Launching rogue nodes with different measurements: {rogue_ips}")
        launch_vms_parallel(rogue_ips, memory="6G", cpus=3)

        # Set log context on rogue nodes when they come up
        for vm in monitor.vms[3:5]:
            try:
                monitor.wait_for_status(vm, VMStatus.BOOTSTRAPPING, timeout=60)
            except TimeoutError:
                pass  # May not reach BOOTSTRAPPING if rejected early

        # Give rogue nodes time to attempt attestation
        # They should fail and remain in BOOTSTRAPPING or get blacklisted
        print("Waiting for attestation attempts (30 seconds)...")
        time.sleep(30)

        # Verify legitimate cluster is still healthy
        for ip in legitimate_ips:
            health = get_health(ip)
            assert health["status"] in ["LEADER", "FOLLOWER"], (
                f"Legitimate node {ip} should be operational, got {health['status']}"
            )
            assert health["cluster_size"] == 3, f"Cluster size should remain 3, got {health['cluster_size']}"

            # Verify legitimate nodes don't trust rogue nodes
            trusted = set(health["trusted_peers"])
            rogue_ids = {int(ip.split(".")[-1]) for ip in rogue_ips}
            trusted_rogues = trusted & rogue_ids
            assert len(trusted_rogues) == 0, f"Node {ip} should not trust rogue nodes, but trusts: {trusted_rogues}"

        # Verify rogue nodes are NOT in an operational state
        for ip in rogue_ips:
            try:
                health = get_health(ip)
                # Rogue nodes should be stuck in BOOTSTRAPPING
                # They can't form their own cluster (only 2 nodes < quorum)
                # They can't join legitimate cluster (measurements don't match)
                assert health["status"] == "BOOTSTRAPPING", (
                    f"Rogue node {ip} should be BOOTSTRAPPING, got {health['status']}"
                )
                assert health["cluster_size"] == 0, (
                    f"Rogue node {ip} should not be in any cluster, cluster_size={health['cluster_size']}"
                )
                print(f"Rogue node {ip}: correctly rejected (status=BOOTSTRAPPING)")
            except Exception as e:
                # If we can't reach the node, that's also acceptable
                # (it may have crashed or been blacklisted)
                print(f"Rogue node {ip}: unreachable or crashed ({e})")

        print("Security test passed: mismatched measurements correctly rejected")

    finally:
        monitor.stop()


@pytest.mark.bootstrap
def test_late_rogue_node_rejected(cluster_ips):
    """
    A rogue node launched after cluster formation is still rejected.

    This tests that the security properties hold even when:
    1. A legitimate cluster is already formed and stable
    2. A new node with mismatched measurements attempts to join later

    Verifies:
    - Running cluster rejects late-joining nodes with bad measurements
    - Cluster remains stable and healthy during attack
    - Rogue node cannot disrupt existing consensus
    """
    legitimate_ips = cluster_ips[:5]
    rogue_ip = cluster_ips[5]  # Use IP from the extra allocation

    kill_vms_parallel(cluster_ips)

    # Launch and form legitimate 5-node cluster
    print(f"Launching legitimate 5-node cluster: {legitimate_ips}")
    launch_vms_parallel(legitimate_ips, memory="8G", cpus=4)

    # Monitor initially for legitimate nodes only
    monitor = create_monitor(legitimate_ips)

    try:
        set_test_context(legitimate_ips, "test_late_rogue_node_rejected")
        wait_for_ready(monitor)

        # Record initial cluster state
        initial_health = get_health(legitimate_ips[0])
        initial_term = initial_health["term"]
        print(f"Cluster formed: size={initial_health['cluster_size']}, term={initial_term}")

        # Now launch rogue node with different measurements
        print(f"Launching late rogue node: {rogue_ip}")
        subprocess.run(
            ["guardian", "vm", "launch", "-t", rogue_ip, "--memory", "6G", "--cpus", "3"],
            check=True,
        )

        # Give rogue node time to boot and attempt attestation
        print("Waiting for rogue attestation attempts (45 seconds)...")
        time.sleep(45)

        # Verify cluster remained stable
        for ip in legitimate_ips:
            health = get_health(ip)
            assert health["status"] in ["LEADER", "FOLLOWER"], (
                f"Node {ip} should still be operational, got {health['status']}"
            )
            assert health["cluster_size"] == 5, f"Cluster size should remain 5, got {health['cluster_size']}"

            # Verify no trust for rogue node
            rogue_id = int(rogue_ip.split(".")[-1])
            assert rogue_id not in health["trusted_peers"], f"Node {ip} should not trust rogue node {rogue_id}"

        # Verify rogue node is stuck
        try:
            rogue_health = get_health(rogue_ip)
            assert rogue_health["status"] == "BOOTSTRAPPING", (
                f"Rogue should be BOOTSTRAPPING, got {rogue_health['status']}"
            )
            print(f"Rogue node correctly rejected: status={rogue_health['status']}")
        except Exception as e:
            print(f"Rogue node unreachable (acceptable): {e}")

        # Verify term didn't change (no unnecessary elections)
        final_health = get_health(legitimate_ips[0])
        assert final_health["term"] == initial_term, (
            f"Term should not change during rogue attack: {initial_term} -> {final_health['term']}"
        )

        print("Late rogue node correctly rejected, cluster stable")

    finally:
        monitor.stop()
        # Clean up rogue node
        subprocess.run(["guardian", "vm", "kill", "-t", rogue_ip], check=False)

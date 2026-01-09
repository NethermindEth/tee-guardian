"""Progressive bootstrap tests for cluster growth scenarios.

Tests cluster expansion from minimum quorum (3 nodes) to larger sizes.
The RAFT coordinator requires BOOTSTRAP_QUORUM=3 mutually-attested nodes
before it will form a cluster (see crates/consensus/src/coordinator.rs).
"""

import subprocess
import time

import pytest
import requests

from guardian_cli.utils.vm_monitor import VMState, VMStatus
from tests.helpers import (
    assert_cluster_size,
    assert_consistent_term,
    assert_mutual_trust,
    assert_single_leader,
    create_monitor,
    get_cluster_state,
    get_health,
    kill_vms_parallel,
    launch_vms_parallel,
    set_test_context,
)


@pytest.mark.bootstrap
def test_progressive_bootstrap(cluster_ips):
    """
    Progressive cluster growth: 3 -> 4 -> 5 nodes.

    This tests the coordinator's ability to:
    1. Bootstrap a minimum 3-node cluster
    2. Add new trusted peers to an existing cluster
    3. Maintain cluster health during expansion

    Verifies:
    - Initial 3-node cluster forms correctly
    - New nodes join as FOLLOWER (not LEADER)
    - Cluster size increments properly
    - All nodes maintain consistent term throughout
    """
    test_ips = cluster_ips[:5]
    kill_vms_parallel(cluster_ips)

    initial_size = 3
    monitor = create_monitor(test_ips[:initial_size])
    launch_vms_parallel(test_ips[:initial_size])

    try:
        set_test_context(test_ips[:initial_size], "test_progressive_bootstrap")
        operational = (VMStatus.LEADER, VMStatus.FOLLOWER, VMStatus.CANDIDATE)
        for vm in monitor.vms:
            monitor.wait_for_status(vm, operational, timeout=120)

        # Verify initial 3-node cluster
        initial_state = get_cluster_state(test_ips[:initial_size])
        initial_leader = assert_single_leader(initial_state)
        initial_term = assert_consistent_term(initial_state)
        assert_cluster_size(initial_state, expected=3)
        print(f"Initial cluster: leader={initial_leader}, term={initial_term}")

        # Phase 2: Add node 4
        print(f"Adding node 4: {test_ips[3]}")
        subprocess.run(["guardian", "vm", "launch", "-t", test_ips[3]], check=True)

        monitor.vms.append(
            VMState(
                vm_id=f"VM-{test_ips[3].split('.')[-1]}",
                ip=test_ips[3],
                health_url=f"http://{test_ips[3]}:8443/health",
            )
        )
        set_test_context([test_ips[3]], "test_progressive_bootstrap")
        monitor.wait_for_status(monitor.vms[3], VMStatus.FOLLOWER, timeout=60)

        # Verify node 4 joined as follower
        health_4 = get_health(test_ips[3])
        assert health_4["status"] == "FOLLOWER", f"Node 4 should be FOLLOWER, got {health_4['status']}"

        # Phase 3: Add node 5
        print(f"Adding node 5: {test_ips[4]}")
        subprocess.run(["guardian", "vm", "launch", "-t", test_ips[4]], check=True)

        monitor.vms.append(
            VMState(
                vm_id=f"VM-{test_ips[4].split('.')[-1]}",
                ip=test_ips[4],
                health_url=f"http://{test_ips[4]}:8443/health",
            )
        )
        set_test_context([test_ips[4]], "test_progressive_bootstrap")
        monitor.wait_for_status(monitor.vms[4], VMStatus.FOLLOWER, timeout=60)

        # Give cluster a moment to stabilize
        time.sleep(2)

        # Verify final cluster state
        final_state = get_cluster_state(test_ips)
        final_leader = assert_single_leader(final_state)
        final_term = assert_consistent_term(final_state)
        assert_cluster_size(final_state, expected=5)
        assert_mutual_trust(test_ips, final_state)

        # Leader should remain the same (no unnecessary elections)
        assert final_leader == initial_leader, (
            f"Leader should remain stable during expansion: {initial_leader} -> {final_leader}"
        )
        print(f"Final cluster: leader={final_leader}, term={final_term}")

    finally:
        monitor.stop()


def get_gossip_peers(ip: str, timeout: int = 5) -> list[dict]:
    """Get peer advertisements from a node's gossip cache via /guardian_node/peers."""
    response = requests.get(f"http://{ip}:8443/guardian_node/peers", timeout=timeout)
    response.raise_for_status()
    return response.json().get("peers", [])


def wait_for_peer_in_gossip(
    query_ip: str,
    target_ip: str,
    timeout: float = 60.0,
    poll_interval: float = 2.0,
) -> dict:
    """
    Wait for a target IP to appear in a node's gossip peer list.

    Args:
        query_ip: IP of node to query /guardian_node/peers
        target_ip: IP we expect to find in the peer list
        timeout: Max seconds to wait
        poll_interval: Seconds between polls

    Returns:
        The peer advertisement dict for target_ip

    Raises:
        TimeoutError: If target_ip not found in gossip peers within timeout
    """
    deadline = time.time() + timeout

    while time.time() < deadline:
        try:
            peers = get_gossip_peers(query_ip)
            for peer in peers:
                # PeerAdvertisement has api_address as "IP:PORT" format
                api_addr = peer.get("api_address", "")
                peer_ip = api_addr.split(":")[0] if ":" in api_addr else ""
                if peer_ip == target_ip:
                    return peer
        except Exception:
            pass
        time.sleep(poll_interval)

    raise TimeoutError(f"Timeout: {target_ip} not found in {query_ip}'s gossip peers")


@pytest.mark.bootstrap
def test_gossip_peer_discovery(cluster_ips):
    """
    Verify gossip protocol enables nodes NOT in the measurement registry to join,
    and that registry nodes learn about gossip-only nodes.

    The measurement registry only contains .20-.24 IPs. Nodes .25-.29 are NOT
    in the registry and can only discover the cluster via gossip protocol.

    This test:
    1. Launches .20-.22 to form initial 3-node cluster (registry nodes)
    2. Launches .25 (gossip-only node, NOT in registry)
    3. Verifies .25 discovers cluster via gossip and joins as FOLLOWER
    4. Verifies registry node (.22) has .25 in its /guardian_node/peers
       (proves gossip propagated the non-registry node's advertisement)

    Verifies:
    - Gossip protocol broadcasts peer advertisements
    - Nodes not in registry can discover cluster via gossip
    - Registry nodes learn about non-registry nodes via gossip
    - Trust is established bidirectionally
    """
    # Registry nodes: .20-.22 (in measurement registry)
    registry_ips = cluster_ips[:3]  # .20, .21, .22
    # Gossip-only node: .25 (NOT in measurement registry, must use gossip)
    gossip_ip = cluster_ips[5]  # .25

    all_ips = registry_ips + [gossip_ip]

    kill_vms_parallel(cluster_ips)

    # Launch registry nodes first to form cluster
    monitor = create_monitor(registry_ips)
    launch_vms_parallel(registry_ips)

    try:
        set_test_context(registry_ips, "test_gossip_peer_discovery")
        operational = (VMStatus.LEADER, VMStatus.FOLLOWER, VMStatus.CANDIDATE)
        for vm in monitor.vms:
            monitor.wait_for_status(vm, operational, timeout=120)

        # Verify initial cluster formed
        initial_state = get_cluster_state(registry_ips)
        assert_single_leader(initial_state)
        assert_cluster_size(initial_state, expected=3)
        print(f"Initial 3-node cluster formed (registry nodes): {registry_ips}")

        # Now launch gossip-only node (.25)
        # This node is NOT in the measurement registry, so it must:
        # 1. Discover peers via gossip advertisements from .20-.22
        # 2. Attest with those peers
        # 3. Join the cluster
        print(f"Launching gossip-only node: {gossip_ip}")
        subprocess.run(["guardian", "vm", "launch", "-t", gossip_ip], check=True)

        # Add gossip node to monitor
        monitor.vms.append(
            VMState(
                vm_id=f"VM-{gossip_ip.split('.')[-1]}",
                ip=gossip_ip,
                health_url=f"http://{gossip_ip}:8443/health",
            )
        )
        set_test_context([gossip_ip], "test_gossip_peer_discovery")

        # Wait for gossip node to join cluster as FOLLOWER
        # This proves gossip worked - the node couldn't have found the cluster otherwise
        gossip_vm = monitor.vms[-1]
        monitor.wait_for_status(gossip_vm, VMStatus.FOLLOWER, timeout=120)
        print(f"Gossip node {gossip_ip} joined cluster as FOLLOWER")

        # Verify final cluster state
        final_state = get_cluster_state(all_ips)
        assert_single_leader(final_state)
        assert_cluster_size(final_state, expected=4)

        # Verify mutual trust between all nodes (registry + gossip)
        # This proves bidirectional attestation worked
        assert_mutual_trust(all_ips, final_state)

        # Verify gossip node trusts exactly 3 peers (the registry nodes)
        gossip_health = final_state.health[gossip_ip]
        assert gossip_health["status"] == "FOLLOWER", f"Gossip node should be FOLLOWER, got {gossip_health['status']}"
        assert len(gossip_health["trusted_peers"]) == 3, (
            f"Gossip node should trust 3 peers, trusts {len(gossip_health['trusted_peers'])}"
        )

        # KEY TEST: Verify registry node (.22) has learned about .25 via gossip
        # This proves gossip propagated the non-registry node's advertisement back
        # to nodes that wouldn't have discovered it via the central registry
        registry_node = registry_ips[2]  # .22
        print(f"Verifying {registry_node} has {gossip_ip} in gossip peers...")
        peer_ad = wait_for_peer_in_gossip(registry_node, gossip_ip, timeout=30)
        print(f"Registry node {registry_node} learned about {gossip_ip} via gossip: {peer_ad.get('api_address')}")

        print(f"Gossip test passed: {gossip_ip} discovered cluster and was propagated via gossip")

    finally:
        monitor.stop()

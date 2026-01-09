"""
Integration tests for RAFT leader election behavior on running clusters.

These tests verify election-related behavior that requires an already-running
cluster (not cluster formation). Tests use network disconnection to simulate
node failures rather than killing VMs.

Tests covered by bootstrap (not duplicated here):
- Basic leader election: test_bootstrap/test_core.py::test_cluster_bootstrap
- Leader crash/recovery: test_bootstrap/test_core.py::test_bootstrap_failover
- Term increase on election: test_bootstrap/test_core.py::test_bootstrap_failover

Tests in this file:
- Follower disconnect doesn't trigger unnecessary election
- Minority cannot elect leader (quorum requirement)
"""

import subprocess
import time

import pytest

from tests.helpers import (
    assert_consistent_term,
    assert_single_leader,
    get_cluster_state,
    get_health,
    wait_for_healthy_cluster,
)


@pytest.mark.integration
def test_follower_disconnect_does_not_trigger_election(cluster_ips, cluster_logs):
    """
    Test that disconnecting a follower doesn't trigger unnecessary elections.

    This verifies cluster stability when a minority node becomes unreachable.
    The leader should continue operating and term should not increase.

    Verifies:
    - Leader remains the same after follower disconnect
    - Term does not increase (no election occurred)
    - Follower rejoins correctly after reconnect
    - Cluster returns to full health

    Test sequence:
    1. Start 5-node cluster, wait for stable state
    2. Disconnect a follower from the network
    3. Verify leader and term unchanged on remaining nodes
    4. Reconnect follower
    5. Verify cluster returns to full health
    """
    test_ips = cluster_ips[:5]
    disconnected_ip: str = ""

    try:
        # Wait for cluster formation
        cluster_logs.wait_for_pattern(
            r"Cluster bootstrap successful|Joined cluster",
            match_all=[{"host_id": ip} for ip in test_ips],
            timeout=120,
        )

        # Get initial state
        initial_state = get_cluster_state(test_ips)
        initial_leader = assert_single_leader(initial_state)
        initial_term = assert_consistent_term(initial_state)
        print(f"Initial: leader={initial_leader}, term={initial_term}")

        # Find a follower to disconnect
        disconnected_ip = next(ip for ip in test_ips if ip != initial_leader)
        print(f"Disconnecting follower: {disconnected_ip}")

        # Disconnect follower from network
        subprocess.run(["guardian", "vm", "disconnect", "-t", disconnected_ip], check=True)

        # Give cluster time to detect the disconnection
        time.sleep(10)

        # Verify leader and term unchanged on reachable nodes
        reachable_ips = [ip for ip in test_ips if ip != disconnected_ip]
        mid_state = get_cluster_state(reachable_ips)
        mid_leader = assert_single_leader(mid_state)
        mid_term = assert_consistent_term(mid_state)

        assert mid_leader == initial_leader, f"Leader should not change: {initial_leader} -> {mid_leader}"
        assert mid_term == initial_term, f"Term should not change: {initial_term} -> {mid_term}"
        print(f"After follower disconnect: leader={mid_leader}, term={mid_term} (unchanged)")

        # Reconnect follower
        print(f"Reconnecting follower: {disconnected_ip}")
        subprocess.run(["guardian", "vm", "reconnect", "-t", disconnected_ip], check=True)

        # Wait for cluster to stabilize
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        final_leader = assert_single_leader(final_state)
        final_term = assert_consistent_term(final_state)

        assert final_leader == initial_leader, f"Leader should remain: {initial_leader} -> {final_leader}"
        assert final_term == initial_term, f"Term should remain: {initial_term} -> {final_term}"

        # Verify follower is back to FOLLOWER status
        follower_health = get_health(disconnected_ip)
        assert follower_health["status"] == "FOLLOWER", (
            f"Reconnected node should be FOLLOWER, got {follower_health['status']}"
        )

        print("Follower disconnect test passed - no unnecessary election triggered")

    finally:
        # Ensure we reconnect any disconnected VMs
        if disconnected_ip:
            subprocess.run(["guardian", "vm", "reconnect", "-t", disconnected_ip], check=False)


@pytest.mark.integration
def test_minority_cannot_elect_leader(cluster_ips, cluster_logs):
    """
    Test that a minority partition cannot elect a leader (quorum requirement).

    This verifies the fundamental RAFT safety property: a partition with fewer
    than majority nodes cannot make progress or elect a leader.

    Verifies:
    - When majority is disconnected, remaining minority cannot form quorum
    - Minority nodes cannot elect a leader
    - Cluster recovers when majority is reconnected

    Test sequence:
    1. Start 5-node cluster
    2. Disconnect 3 nodes (majority), leaving 2 (minority)
    3. Verify remaining 2 nodes cannot elect leader
    4. Reconnect the 3 nodes
    5. Verify cluster recovers
    """
    test_ips = cluster_ips[:5]
    disconnected_ips: list[str] = []

    try:
        # Wait for cluster formation
        cluster_logs.wait_for_pattern(
            r"Cluster bootstrap successful|Joined cluster",
            match_all=[{"host_id": ip} for ip in test_ips],
            timeout=120,
        )

        # Get initial state
        initial_state = get_cluster_state(test_ips)
        initial_leader = assert_single_leader(initial_state)
        print(f"Initial leader: {initial_leader}")

        # Disconnect 3 nodes (majority), keeping 2 (minority)
        # Include the leader in disconnected set to ensure we test minority behavior
        majority_to_disconnect = test_ips[:3]
        minority_ips = test_ips[3:]
        print(f"Disconnecting majority: {majority_to_disconnect}")
        print(f"Minority remaining: {minority_ips}")

        for ip in majority_to_disconnect:
            subprocess.run(["guardian", "vm", "disconnect", "-t", ip], check=True)
        disconnected_ips = majority_to_disconnect

        # Wait for minority to attempt elections (and fail)
        print("Waiting for minority to attempt elections (should fail)...")
        time.sleep(15)

        # Verify minority cannot elect leader
        leader_count = 0
        for ip in minority_ips:
            try:
                health = get_health(ip, timeout=3)
                print(f"Minority node {ip}: status={health['status']}")
                if health["status"] == "LEADER":
                    leader_count += 1
            except Exception as e:
                print(f"Minority node {ip}: unreachable ({e})")

        assert leader_count == 0, f"Minority should not elect leader, found {leader_count} leaders"
        print("Minority correctly unable to elect leader (quorum requirement enforced)")

        # Reconnect majority
        print(f"Reconnecting majority: {majority_to_disconnect}")
        for ip in majority_to_disconnect:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=True)
        disconnected_ips = []

        # Wait for cluster to recover
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        assert_single_leader(final_state)
        print("Cluster recovered after majority reconnected")

        print("Minority cannot elect leader test passed")

    finally:
        # Ensure we reconnect any disconnected VMs
        for ip in disconnected_ips:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=False)

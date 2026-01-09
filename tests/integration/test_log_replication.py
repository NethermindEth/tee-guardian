"""
Integration tests for RAFT log replication and state synchronization.

These tests verify state synchronization behavior that requires an already-running
cluster. Tests use network disconnection to simulate node isolation rather than
killing VMs.

Tests covered by bootstrap (not duplicated here):
- Cluster state consistency: test_bootstrap/test_core.py::test_cluster_bootstrap
- Follower catches up after restart: test_bootstrap/test_core.py::test_bootstrap_failover
- Trusted peer list consistency: test_bootstrap/test_core.py::test_cluster_bootstrap

Tests in this file:
- Lagging follower eventually syncs after missing term bumps
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
    wait_for_new_leader,
)


@pytest.mark.integration
def test_lagging_follower_eventually_syncs(cluster_ips, cluster_logs):
    """
    Test that a follower that was isolated during elections eventually syncs up.

    This test verifies that a node isolated from the cluster during term changes
    will correctly synchronize its state when reconnected, even if it missed
    multiple elections.

    Verifies:
    - Isolated follower misses term increase
    - After reconnection, follower syncs to current term
    - All nodes converge to consistent state
    - No permanent desynchronization

    Test sequence:
    1. Start 5-node cluster, record initial term
    2. Disconnect a follower (it will miss subsequent events)
    3. Disconnect the leader to force a term bump
    4. Reconnect the old leader (not the isolated follower yet)
    5. Verify cluster has higher term
    6. Reconnect the isolated follower
    7. Verify follower catches up to current term
    """
    test_ips = cluster_ips[:5]
    isolated_follower: str = ""
    isolated_leader: str = ""

    try:
        # Wait for cluster formation
        cluster_logs.wait_for_pattern(
            r"Cluster bootstrap successful|Joined cluster",
            match_all=[{"host_id": ip} for ip in test_ips],
            timeout=120,
        )

        initial_state = get_cluster_state(test_ips)
        initial_leader = assert_single_leader(initial_state)
        initial_term = assert_consistent_term(initial_state)
        print(f"Initial: leader={initial_leader}, term={initial_term}")

        # Isolate a follower - it will miss subsequent term changes
        isolated_follower = next(ip for ip in test_ips if ip != initial_leader)
        print(f"Isolating follower (will lag behind): {isolated_follower}")
        subprocess.run(["guardian", "vm", "disconnect", "-t", isolated_follower], check=True)

        # Give cluster time to detect isolation
        time.sleep(5)

        # Now disconnect the leader to cause a term bump
        # The remaining 3 nodes should elect a new leader with higher term
        remaining_connected = [ip for ip in test_ips if ip not in [isolated_follower, initial_leader]]
        print(f"Disconnecting leader to force term bump: {initial_leader}")
        subprocess.run(["guardian", "vm", "disconnect", "-t", initial_leader], check=True)
        isolated_leader = initial_leader

        # Wait for new leader election among remaining 3 nodes
        new_leader = wait_for_new_leader(remaining_connected, initial_leader, timeout=30)
        print(f"New leader elected: {new_leader}")

        # Verify term increased
        mid_state = get_cluster_state(remaining_connected)
        mid_term = assert_consistent_term(mid_state)
        assert mid_term > initial_term, f"Term should have increased: {initial_term} -> {mid_term}"
        print(f"Current term: {mid_term} (isolated follower still at {initial_term})")

        # Reconnect old leader first - it should rejoin as follower
        print(f"Reconnecting old leader: {isolated_leader}")
        subprocess.run(["guardian", "vm", "reconnect", "-t", isolated_leader], check=True)
        isolated_leader = ""

        time.sleep(5)

        # Verify old leader synced up
        old_leader_health = get_health(initial_leader)
        assert old_leader_health["status"] == "FOLLOWER", (
            f"Old leader should be FOLLOWER, got {old_leader_health['status']}"
        )
        print(f"Old leader rejoined as FOLLOWER with term {old_leader_health['term']}")

        # Now reconnect the isolated follower
        print(f"Reconnecting isolated follower: {isolated_follower}")
        subprocess.run(["guardian", "vm", "reconnect", "-t", isolated_follower], check=True)
        isolated_follower = ""

        # Wait for cluster to stabilize
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        final_term = assert_consistent_term(final_state)
        print(f"Final cluster term: {final_term}")

        # Verify all nodes have the same term
        for ip in test_ips:
            health = final_state.health[ip]
            assert health["term"] == final_term, f"Node {ip} has term {health['term']}, expected {final_term}"

        print(f"All nodes synced to term {final_term}")
        print("Lagging follower sync test passed")

    finally:
        # Ensure we reconnect any disconnected VMs
        for ip in [isolated_follower, isolated_leader]:
            if ip:
                subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=False)

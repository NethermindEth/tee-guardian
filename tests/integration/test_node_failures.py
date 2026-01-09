"""
Integration tests for node failure scenarios (simulated via network isolation).

These tests verify cluster behavior when nodes become unreachable. Network
isolation is used to simulate node failures, allowing the test to verify
recovery without needing to restart VMs.

Tests covered by bootstrap (not duplicated here):
- Follower crash and restart: test_bootstrap/test_core.py::test_bootstrap_failover
- Leader crash and restart: test_bootstrap/test_core.py::test_bootstrap_failover

Tests in this file:
- Multiple follower failures (maintain quorum)
- Majority failure blocks cluster progress
- Repeated failures of same node
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
    wait_for_leader,
)


@pytest.mark.integration
def test_multiple_follower_failures(cluster_ips, cluster_logs):
    """
    Test that cluster survives multiple follower failures (minority failure).

    This verifies that when multiple followers become unreachable but quorum
    is maintained (leader + 2 followers = 3/5), the cluster continues operating.

    Verifies:
    - Cluster survives when minority of followers fail
    - Leader remains stable
    - Cluster continues to have quorum (3 of 5)
    - Full recovery when nodes return

    Test sequence:
    1. Start 5-node cluster
    2. Disconnect 2 followers simultaneously
    3. Verify cluster operational (leader + 2 remaining followers)
    4. Reconnect both followers
    5. Verify full recovery
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

        initial_state = get_cluster_state(test_ips)
        initial_leader = assert_single_leader(initial_state)
        initial_term = assert_consistent_term(initial_state)
        print(f"Initial: leader={initial_leader}, term={initial_term}")

        # Disconnect 2 followers (keeping leader + 2 others for quorum)
        followers = [ip for ip in test_ips if ip != initial_leader]
        nodes_to_fail = followers[:2]
        surviving_ips = [initial_leader] + followers[2:]

        print(f"Failing 2 followers: {nodes_to_fail}")
        print(f"Surviving nodes: {surviving_ips}")
        for ip in nodes_to_fail:
            subprocess.run(["guardian", "vm", "disconnect", "-t", ip], check=True)
        disconnected_ips = list(nodes_to_fail)

        # Give cluster time to detect failures
        time.sleep(10)

        # Verify cluster still operational with 3 nodes (quorum)
        mid_state = get_cluster_state(surviving_ips)
        mid_leader = assert_single_leader(mid_state)
        mid_term = assert_consistent_term(mid_state)

        assert mid_leader == initial_leader, f"Leader should remain: {initial_leader} -> {mid_leader}"
        assert mid_term == initial_term, f"Term should not change: {initial_term} -> {mid_term}"
        print(f"Cluster operational with 3 nodes: leader={mid_leader}, term={mid_term}")

        # Reconnect both followers
        print(f"Reconnecting followers: {nodes_to_fail}")
        for ip in nodes_to_fail:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=True)
        disconnected_ips = []

        # Wait for full recovery
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        assert_single_leader(final_state)
        print("Full cluster recovery achieved")

        print("Multiple follower failures test passed")

    finally:
        for ip in disconnected_ips:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=False)


@pytest.mark.integration
def test_majority_failure_blocks_cluster(cluster_ips, cluster_logs):
    """
    Test that majority failure blocks cluster operation.

    This verifies the fundamental RAFT property that a cluster cannot make
    progress without quorum. When majority is unreachable, remaining nodes
    cannot elect a leader.

    Verifies:
    - Cluster loses quorum when majority fails
    - Remaining minority cannot elect leader
    - Cluster recovers when enough nodes return to form quorum

    Test sequence:
    1. Start 5-node cluster
    2. Disconnect 3 nodes (majority)
    3. Verify remaining 2 nodes cannot elect leader
    4. Reconnect 1 node to restore quorum
    5. Verify cluster recovers with 3 nodes
    6. Reconnect remaining nodes
    7. Verify full recovery
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

        initial_state = get_cluster_state(test_ips)
        assert_single_leader(initial_state)

        # Disconnect 3 nodes (majority)
        nodes_to_fail = test_ips[:3]
        minority_ips = test_ips[3:]
        print(f"Failing majority: {nodes_to_fail}")
        print(f"Minority remaining: {minority_ips}")
        for ip in nodes_to_fail:
            subprocess.run(["guardian", "vm", "disconnect", "-t", ip], check=True)
        disconnected_ips = list(nodes_to_fail)

        # Wait for minority to attempt elections
        print("Waiting for minority to attempt elections...")
        time.sleep(15)

        # Verify no leader in minority
        leader_count = 0
        for ip in minority_ips:
            try:
                health = get_health(ip, timeout=3)
                if health["status"] == "LEADER":
                    leader_count += 1
                print(f"Minority node {ip}: status={health['status']}")
            except Exception as e:
                print(f"Minority node {ip}: error={e}")

        assert leader_count == 0, f"Minority should not elect leader, found {leader_count}"
        print("Minority correctly cannot elect leader (quorum lost)")

        # Reconnect 1 node to restore quorum (2 minority + 1 = 3 = quorum)
        node_to_restore = nodes_to_fail[0]
        print(f"Reconnecting one node to restore quorum: {node_to_restore}")
        subprocess.run(["guardian", "vm", "reconnect", "-t", node_to_restore], check=True)
        disconnected_ips.remove(node_to_restore)

        # Wait for cluster to recover with 3 nodes
        active_ips = minority_ips + [node_to_restore]
        wait_for_leader(active_ips, timeout=30)
        print("Cluster recovered with quorum (3 nodes), leader elected")

        # Reconnect remaining nodes
        remaining = disconnected_ips[:]
        if remaining:
            print(f"Reconnecting remaining nodes: {remaining}")
            for ip in remaining:
                subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=True)
            disconnected_ips = []

        # Verify full recovery
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        assert_single_leader(final_state)
        print("Full cluster recovery achieved")

        print("Majority failure test passed")

    finally:
        for ip in disconnected_ips:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=False)


@pytest.mark.integration
def test_repeated_node_failures(cluster_ips, cluster_logs):
    """
    Test that a node can fail and recover multiple times without corruption.

    This verifies cluster stability when the same node experiences repeated
    network failures. The cluster should handle this gracefully without
    state corruption or instability.

    Verifies:
    - Node can become unreachable multiple times
    - Cluster remains stable during repeated failures
    - No corruption from repeated failures
    - Node correctly rejoins each time

    Test sequence:
    1. Start 5-node cluster
    2. Disconnect a follower
    3. Reconnect the follower
    4. Repeat steps 2-3 multiple times
    5. Verify cluster health after each cycle
    """
    test_ips = cluster_ips[:5]
    target_ip: str = ""

    try:
        # Wait for cluster formation
        cluster_logs.wait_for_pattern(
            r"Cluster bootstrap successful|Joined cluster",
            match_all=[{"host_id": ip} for ip in test_ips],
            timeout=120,
        )

        initial_state = get_cluster_state(test_ips)
        initial_leader = assert_single_leader(initial_state)
        print(f"Initial leader: {initial_leader}")

        # Select a follower to repeatedly fail
        target_ip = next(ip for ip in test_ips if ip != initial_leader)
        print(f"Target for repeated failures: {target_ip}")

        for iteration in range(3):
            print(f"\n--- Failure iteration {iteration + 1} ---")

            # Disconnect target
            print(f"Disconnecting: {target_ip}")
            subprocess.run(["guardian", "vm", "disconnect", "-t", target_ip], check=True)

            # Give cluster time to detect
            time.sleep(5)

            # Verify cluster still operational without target
            remaining_ips = [ip for ip in test_ips if ip != target_ip]
            mid_state = get_cluster_state(remaining_ips)
            assert_single_leader(mid_state)
            print("Cluster operational without target")

            # Reconnect target
            print(f"Reconnecting: {target_ip}")
            subprocess.run(["guardian", "vm", "reconnect", "-t", target_ip], check=True)

            # Wait for target to rejoin
            time.sleep(5)

            # Verify target rejoined correctly
            target_health = get_health(target_ip)
            assert target_health["status"] == "FOLLOWER", (
                f"Iteration {iteration + 1}: target should be FOLLOWER, got {target_health['status']}"
            )

            state = get_cluster_state(test_ips)
            assert_single_leader(state)
            print(f"Iteration {iteration + 1}: cluster healthy, target rejoined as FOLLOWER")

        # Final verification
        final_state = wait_for_healthy_cluster(test_ips, timeout=30)
        assert_single_leader(final_state)
        print("\nRepeated node failures test passed")

    finally:
        # Ensure target is reconnected
        if target_ip:
            subprocess.run(["guardian", "vm", "reconnect", "-t", target_ip], check=False)

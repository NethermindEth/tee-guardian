"""
Integration tests for network partition scenarios.

These tests verify cluster behavior during actual network partitions using
iptables-based network isolation (guardian vm disconnect/reconnect).

Tests covered by bootstrap (not duplicated here):
- Partition healing and node rejoin: test_bootstrap/test_core.py::test_bootstrap_failover

Tests in this file:
- Majority partition continues operation when minority isolated
- Split-brain prevention (no two leaders in same term)
- Cascading failures and recovery
"""

import subprocess
import time

import pytest

from tests.helpers import (
    assert_consistent_term,
    assert_single_leader,
    get_cluster_state,
    wait_for_healthy_cluster,
    wait_for_new_leader,
)


@pytest.mark.integration
def test_majority_continues_operation(cluster_ips, cluster_logs):
    """
    Test that majority partition continues normal operation when minority isolated.

    This verifies that when followers are partitioned away, the majority
    (including the leader) continues operating without disruption.

    Verifies:
    - Majority maintains leader
    - Majority can continue operation (term unchanged)
    - Recovery when minority returns

    Test sequence:
    1. Start 5-node cluster
    2. Disconnect 2 followers (minority)
    3. Verify leader remains stable, term unchanged
    4. Reconnect minority
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

        # Disconnect 2 followers (minority), keeping leader + 2 others (majority)
        followers = [ip for ip in test_ips if ip != initial_leader]
        minority_to_isolate = followers[:2]
        majority_ips = [ip for ip in test_ips if ip not in minority_to_isolate]

        print(f"Isolating minority: {minority_to_isolate}")
        print(f"Majority remaining: {majority_ips}")
        for ip in minority_to_isolate:
            subprocess.run(["guardian", "vm", "disconnect", "-t", ip], check=True)
        disconnected_ips = minority_to_isolate

        # Give cluster time to detect partition
        time.sleep(10)

        # Verify majority still operational
        mid_state = get_cluster_state(majority_ips)
        mid_leader = assert_single_leader(mid_state)
        mid_term = assert_consistent_term(mid_state)

        assert mid_leader == initial_leader, f"Majority leader should remain: {initial_leader} -> {mid_leader}"
        assert mid_term == initial_term, f"Term should not change: {initial_term} -> {mid_term}"
        print(f"Majority operational: leader={mid_leader}, term={mid_term} (unchanged)")

        # Reconnect minority
        print(f"Reconnecting minority: {minority_to_isolate}")
        for ip in minority_to_isolate:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=True)
        disconnected_ips = []

        # Wait for full recovery
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        final_leader = assert_single_leader(final_state)
        final_term = assert_consistent_term(final_state)

        # Leader should remain stable through the whole process
        assert final_leader == initial_leader, f"Leader should remain: {initial_leader} -> {final_leader}"
        print(f"Full recovery: leader={final_leader}, term={final_term}")

        print("Majority continues operation test passed")

    finally:
        for ip in disconnected_ips:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=False)


@pytest.mark.integration
def test_split_brain_prevention(cluster_ips, cluster_logs):
    """
    Test that split-brain (two leaders in same term) never occurs.

    This test repeatedly isolates the leader, waits for new election,
    and verifies that at no point do we have multiple leaders claiming
    the same term.

    Verifies:
    - At most one leader at any time
    - No two leaders in same term (critical safety property)
    - Terms increase monotonically with elections

    Test sequence:
    1. Start 5-node cluster
    2. Isolate leader, wait for new election
    3. Verify term increased, single leader
    4. Reconnect old leader
    5. Repeat 2-4 multiple times
    6. Verify no split-brain occurred
    """
    test_ips = cluster_ips[:5]
    disconnected_leader: str = ""

    try:
        # Wait for cluster formation
        cluster_logs.wait_for_pattern(
            r"Cluster bootstrap successful|Joined cluster",
            match_all=[{"host_id": ip} for ip in test_ips],
            timeout=120,
        )

        # Track term progression
        terms_seen: list[int] = []
        split_brain_detected = False

        for iteration in range(3):
            print(f"\n--- Iteration {iteration + 1} ---")

            # Get current state and check for split-brain
            state = get_cluster_state(test_ips)

            # Check leader count
            leaders = [ip for ip, h in state.health.items() if h["status"] == "LEADER"]
            if len(leaders) > 1:
                # Check if they're in the same term (true split-brain)
                leader_terms = {state.health[ip]["term"] for ip in leaders}
                if len(leader_terms) == 1:
                    split_brain_detected = True
                    pytest.fail(f"SPLIT-BRAIN DETECTED: Multiple leaders {leaders} in term {leader_terms}")

            leader = assert_single_leader(state)
            term = assert_consistent_term(state)
            terms_seen.append(term)
            print(f"Leader: {leader}, Term: {term}")

            # Disconnect leader to force new election
            print(f"Isolating leader: {leader}")
            subprocess.run(["guardian", "vm", "disconnect", "-t", leader], check=True)
            disconnected_leader = leader

            # Wait for new election among remaining nodes
            remaining_ips = [ip for ip in test_ips if ip != leader]
            new_leader = wait_for_new_leader(remaining_ips, leader, timeout=30)
            new_state = get_cluster_state(remaining_ips)
            new_term = assert_consistent_term(new_state)

            assert new_term > term, f"Term must increase: {term} -> {new_term}"
            terms_seen.append(new_term)
            print(f"New leader: {new_leader}, New term: {new_term}")

            # Reconnect old leader
            print(f"Reconnecting old leader: {leader}")
            subprocess.run(["guardian", "vm", "reconnect", "-t", leader], check=True)
            disconnected_leader = ""

            # Wait for cluster to stabilize
            time.sleep(5)

        # Final verification
        final_state = get_cluster_state(test_ips)
        assert_single_leader(final_state)

        # Verify terms increased monotonically
        for i in range(1, len(terms_seen)):
            assert terms_seen[i] >= terms_seen[i - 1], f"Terms should not decrease: {terms_seen}"

        assert not split_brain_detected, "Split-brain was detected during test"
        print(f"\nTerm progression: {terms_seen}")
        print("Split-brain prevention test passed")

    finally:
        if disconnected_leader:
            subprocess.run(["guardian", "vm", "reconnect", "-t", disconnected_leader], check=False)


@pytest.mark.integration
def test_cascading_failures_recovery(cluster_ips, cluster_logs):
    """
    Test recovery from multiple sequential network failures.

    This simulates cascading network issues where multiple nodes become
    isolated one after another, then recover in sequence.

    Verifies:
    - Cluster survives sequential partitions (as long as quorum maintained)
    - Each partition handled correctly
    - Full recovery when all nodes reconnected

    Test sequence:
    1. Start 5-node cluster
    2. Disconnect first follower, verify cluster stable
    3. Disconnect second follower, verify cluster stable (3 nodes = quorum)
    4. Reconnect first follower
    5. Reconnect second follower
    6. Verify full recovery
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
        leader = assert_single_leader(initial_state)
        print(f"Initial leader: {leader}")

        # Get two followers to disconnect sequentially
        followers = [ip for ip in test_ips if ip != leader]
        victim1 = followers[0]
        victim2 = followers[1]

        # First failure
        print(f"\n--- First failure: {victim1} ---")
        subprocess.run(["guardian", "vm", "disconnect", "-t", victim1], check=True)
        disconnected_ips.append(victim1)

        time.sleep(5)
        remaining1 = [ip for ip in test_ips if ip != victim1]
        state1 = get_cluster_state(remaining1)
        assert_single_leader(state1)
        print("Cluster stable with 4 connected nodes")

        # Second failure
        print(f"\n--- Second failure: {victim2} ---")
        subprocess.run(["guardian", "vm", "disconnect", "-t", victim2], check=True)
        disconnected_ips.append(victim2)

        time.sleep(5)
        remaining2 = [ip for ip in test_ips if ip not in disconnected_ips]
        state2 = get_cluster_state(remaining2)
        assert_single_leader(state2)
        print("Cluster stable with 3 connected nodes (quorum maintained)")

        # First recovery
        print(f"\n--- First recovery: {victim1} ---")
        subprocess.run(["guardian", "vm", "reconnect", "-t", victim1], check=True)
        disconnected_ips.remove(victim1)

        time.sleep(5)
        remaining3 = [ip for ip in test_ips if ip not in disconnected_ips]
        state3 = get_cluster_state(remaining3)
        assert_single_leader(state3)
        print("Cluster stable with 4 connected nodes")

        # Second recovery
        print(f"\n--- Second recovery: {victim2} ---")
        subprocess.run(["guardian", "vm", "reconnect", "-t", victim2], check=True)
        disconnected_ips.remove(victim2)

        # Verify full recovery
        final_state = wait_for_healthy_cluster(test_ips, timeout=60)
        assert_single_leader(final_state)
        print("Full cluster recovery achieved")

        print("\nCascading failures recovery test passed")

    finally:
        for ip in disconnected_ips:
            subprocess.run(["guardian", "vm", "reconnect", "-t", ip], check=False)

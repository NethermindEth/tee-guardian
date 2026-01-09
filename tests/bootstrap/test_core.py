"""Bootstrap tests for cluster initialization and RAFT consensus verification."""

import subprocess
import time

import pytest

from guardian_cli.utils.vm_monitor import VMStatus
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
    wait_for_ready,
)


@pytest.mark.bootstrap
def test_cluster_bootstrap(cluster_ips):
    """
    5-node cluster forms correctly with proper RAFT consensus.

    Verifies:
    - All 5 nodes reach operational state (LEADER/FOLLOWER/CANDIDATE)
    - Exactly one leader is elected (no split-brain)
    - All nodes agree on the same RAFT term
    - All nodes report correct cluster_size
    - All nodes mutually trust each other (bidirectional attestation)
    """
    test_ips = cluster_ips[:5]

    kill_vms_parallel(cluster_ips)
    launch_vms_parallel(test_ips)

    monitor = create_monitor(test_ips)
    try:
        set_test_context(test_ips, "test_cluster_bootstrap")
        wait_for_ready(monitor)

        # Give cluster a moment to fully stabilize
        time.sleep(2)

        # Get cluster state and run all assertions
        state = get_cluster_state(test_ips)

        # Verify exactly one leader (critical - detects split-brain)
        leader_ip = assert_single_leader(state)
        print(f"Leader: {leader_ip}")

        # Verify all nodes agree on term
        term = assert_consistent_term(state)
        print(f"Term: {term}")

        # Verify cluster size consistency
        assert_cluster_size(state, expected=5)

        # Verify mutual trust (all nodes attested each other)
        assert_mutual_trust(test_ips, state)

        # Verify all non-leaders are followers (not candidates)
        for ip, health in state.health.items():
            if ip != leader_ip:
                assert health["status"] == "FOLLOWER", f"Node {ip} should be FOLLOWER, got {health['status']}"

    finally:
        monitor.stop()


@pytest.mark.bootstrap
def test_bootstrap_failover(cluster_ips):
    """
    Cluster recovers from early bootstrap coordinator loss and subsequent leader failure.

    This test exercises multiple failure scenarios:
    1. Kill bootstrap coordinator (.20) early (after it has peers but before cluster forms)
    2. Wait for remaining nodes to form cluster and elect leader
    3. Kill the elected leader to force a new election
    4. Restart the original .20 node
    5. Verify cluster stabilizes with term progression

    Verifies:
    - Cluster forms despite early loss of bootstrap coordinator
    - Leader election works correctly
    - Term increases after leader death
    - Killed nodes rejoin as followers
    - No split-brain during recovery
    """
    test_ips = cluster_ips[:5]
    bootstrap_ip = test_ips[0]  # .20 is the bootstrap coordinator

    kill_vms_parallel(cluster_ips)
    launch_vms_parallel(test_ips)

    monitor = create_monitor(test_ips)
    try:
        set_test_context(test_ips, "test_bootstrap_failover")

        # Wait for bootstrap coordinator to have at least 1 trusted peer, then kill it
        bootstrap_vm = monitor.vms[0]
        print(f"Waiting for {bootstrap_ip} to discover peers...")

        # Poll until bootstrap node has trusted peers
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                health = get_health(bootstrap_ip, timeout=2)
                if len(health.get("trusted_peers", [])) >= 1:
                    print(f"Bootstrap node has {len(health['trusted_peers'])} trusted peers, killing it")
                    break
            except Exception:
                pass
            time.sleep(1)
        else:
            raise TimeoutError(f"Bootstrap node {bootstrap_ip} never discovered peers")

        # Kill bootstrap coordinator early
        subprocess.run(["guardian", "vm", "kill", "-t", bootstrap_ip], check=True)
        monitor.wait_for_status(bootstrap_vm, VMStatus.STOPPED, timeout=30)
        print(f"Killed bootstrap coordinator: {bootstrap_ip}")

        # Wait for remaining 4 nodes to form cluster
        remaining_ips = [ip for ip in test_ips if ip != bootstrap_ip]
        operational = (VMStatus.LEADER, VMStatus.FOLLOWER, VMStatus.CANDIDATE)
        for vm in monitor.vms[1:]:  # Skip the killed bootstrap VM
            monitor.wait_for_status(vm, operational, timeout=120)

        # Get cluster state after bootstrap coordinator death
        time.sleep(2)  # Let cluster stabilize
        state_after_bootstrap_kill = get_cluster_state(remaining_ips)
        first_leader = assert_single_leader(state_after_bootstrap_kill)
        term_after_bootstrap_kill = assert_consistent_term(state_after_bootstrap_kill)
        print(f"Cluster formed without bootstrap coordinator: leader={first_leader}, term={term_after_bootstrap_kill}")

        # Now kill the elected leader to force a new election
        print(f"Killing leader: {first_leader}")
        subprocess.run(["guardian", "vm", "kill", "-t", first_leader], check=True)
        leader_vm = next(vm for vm in monitor.vms if vm.ip == first_leader)
        monitor.wait_for_status(leader_vm, VMStatus.STOPPED, timeout=30)

        # Wait for new leader election
        surviving_ips = [ip for ip in remaining_ips if ip != first_leader]
        time.sleep(5)  # Allow election timeout

        state_after_leader_kill = get_cluster_state(surviving_ips)
        second_leader = assert_single_leader(state_after_leader_kill)
        term_after_leader_kill = assert_consistent_term(state_after_leader_kill)
        print(f"After leader kill: new_leader={second_leader}, term={term_after_leader_kill}")

        # Term MUST increase after leader death
        assert term_after_leader_kill > term_after_bootstrap_kill, (
            f"Term must increase after leader death: {term_after_bootstrap_kill} -> {term_after_leader_kill}"
        )

        # Restart the original bootstrap coordinator
        print(f"Restarting bootstrap coordinator: {bootstrap_ip}")
        subprocess.run(["guardian", "vm", "launch", "-t", bootstrap_ip], check=True)
        monitor.wait_for_status(bootstrap_vm, (VMStatus.LEADER, VMStatus.FOLLOWER), timeout=120)

        # Give cluster time to stabilize
        time.sleep(3)

        # Get final state (4 nodes: 3 survivors + bootstrap coordinator)
        # Note: killed leader is still down
        active_ips = surviving_ips + [bootstrap_ip]
        final_state = get_cluster_state(active_ips)
        final_leader = assert_single_leader(final_state)
        final_term = assert_consistent_term(final_state)
        print(f"Final state: leader={final_leader}, term={final_term}")

        # Bootstrap coordinator should rejoin as follower
        assert final_state.health[bootstrap_ip]["status"] == "FOLLOWER", (
            f"Rejoined bootstrap node should be FOLLOWER, got {final_state.health[bootstrap_ip]['status']}"
        )

        # Term should be at least what it was after leader kill (may have increased further)
        assert final_term >= term_after_leader_kill, (
            f"Term should not decrease: {term_after_leader_kill} -> {final_term}"
        )

        # Verify mutual trust among active nodes
        assert_mutual_trust(active_ips, final_state)

        print(f"Test passed: term progressed {term_after_bootstrap_kill} -> {term_after_leader_kill} -> {final_term}")

    finally:
        monitor.stop()

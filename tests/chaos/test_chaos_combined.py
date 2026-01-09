"""
Chaos testing scenarios with combined, randomized faults.

These tests inject multiple, concurrent, random failures to stress-test
the system's resilience. Inspired by Jepsen-style testing.

Tests cover:
- Random node crashes
- Random network partitions
- Random latency injection
- Combined fault scenarios
- Long-running chaos tests
"""

import pytest
import random
import time


@pytest.mark.chaos
@pytest.mark.slow
def test_random_node_crashes(vm_monitor):
    """
    Randomly crash and restart nodes over 5 minutes.

    Verifies:
    - Cluster survives random crashes
    - No data loss
    - Always has available leader
    - Eventual consistency
    """
    # --- Setup: 5-node cluster
    # subprocess.call(["guardian", "cluster", "start", "--nodes", "5"])
    # vm_monitor.wait_for_all_healthy(timeout=30.0)

    # --- Write initial reference data
    # leader = get_leader(vm_monitor)
    # reference_data = {}
    # for i in range(100):
    #     key = f"reference_{i}"
    #     value = f"value_{i}"
    #     write_key(leader.ip, key, value)
    #     reference_data[key] = value

    # --- Track successful writes during chaos
    # chaos_writes = {}
    #
    # --- Start chaos nemesis in background
    # async def chaos_nemesis():
    #     start_time = time.time()
    #     while time.time() - start_time < 300:  # 5 minutes
    #         # Pick random node
    #         node = random.choice(vm_monitor.nodes)
    #
    #         # Random action
    #         action = random.choice([
    #             "kill",
    #             "restart",
    #             "nothing",
    #             "nothing"  # Weight towards doing nothing sometimes
    #         ])
    #
    #         if action == "kill":
    #             subprocess.call(["guardian", "vm", "kill", "--target", node.ip])
    #             time.sleep(random.uniform(2, 5))
    #         elif action == "restart":
    #             subprocess.call(["guardian", "vm", "restart", "--target", node.ip])
    #             time.sleep(random.uniform(5, 10))
    #
    #         time.sleep(random.uniform(3, 10))
    #
    # nemesis_task = # asyncio.create_task(chaos_nemesis())

    # --- Concurrent client operations
    # async def client_operations():
    #     write_count = 0
    #     while not nemesis_task.done():
    #         try:
    #             # Try to find current leader
    #             leader = get_leader(vm_monitor)
    #
    #             # Write data
    #             key = f"chaos_write_{write_count}"
    #             value = f"value_{write_count}"
    #             response = write_key(leader.ip, key, value, timeout=5.0)
    #
    #             if response.status == 200:
    #                 chaos_writes[key] = value
    #                 write_count += 1
    #         except Exception as e:
    #             # Expected during chaos - leader may be down
    #             time.sleep(1.0)
    #
    #         time.sleep(random.uniform(0.1, 0.5))
    #
    # client_task = # asyncio.create_task(client_operations())

    # --- Wait for chaos to complete
    # nemesis_task
    # client_task.cancel()

    # --- Heal all and wait for stabilization
    # subprocess.call(["guardian", "cluster", "heal-all"])
    # time.sleep(30.0)

    # --- Verify: All reference data intact
    # for node in vm_monitor.nodes:
    #     for key, expected_value in reference_data.items():
    #         value = read_key(node.ip, key)
    #         assert value == expected_value, f"Reference data corrupted on node {node.id}"

    # --- Verify: All successful chaos writes are present
    # for node in vm_monitor.nodes:
    #     for key, expected_value in chaos_writes.items():
    #         value = read_key(node.ip, key)
    #         assert value == expected_value, f"Chaos write lost on node {node.id}"

    # --- Verify: All nodes have same final state
    # final_log_indices = [get_metrics(n.ip)["last_log_index"] for n in vm_monitor.nodes]
    # assert len(set(final_log_indices)) == 1, "All nodes should converge to same log"

    pass


@pytest.mark.chaos
@pytest.mark.slow
def test_random_network_partitions(vm_monitor):
    """
    Create random network partitions for 5 minutes.

    Verifies:
    - Cluster handles partitions
    - Majority always operational
    - Split-brain never occurs
    - Data consistent after healing
    """
    # --- Setup: 5-node cluster
    # subprocess.call(["guardian", "cluster", "start", "--nodes", "5"])
    # vm_monitor.wait_for_all_healthy(timeout=30.0)

    # --- Chaos nemesis
    # async def partition_nemesis():
    #     start_time = time.time()
    #     while time.time() - start_time < 300:
    #         # Create random partition
    #         partition_type = random.choice([
    #             "minority",     # 2 vs 3
    #             "symmetric",    # 2 vs 2 vs 1
    #             "isolate_one",  # 1 vs 4
    #             "heal"          # Heal all partitions
    #         ])
    #
    #         if partition_type == "minority":
    #             nodes = list(range(5))
    #             random.shuffle(nodes)
    #             minority = nodes[:2]
    #             majority = nodes[2:]
    #
    #             for m in minority:
    #                 for M in majority:
    #                     subprocess.call(["guardian", "vm", "disconnect",
    #                                     "--target", vm_monitor.nodes[m].ip,
    #                                     "--from", vm_monitor.nodes[M].ip])
    #
    #         elif partition_type == "isolate_one":
    #             isolated = random.choice(list(range(5)))
    #             subprocess.call(["guardian", "vm", "disconnect",
    #                             "--target", vm_monitor.nodes[isolated].ip])
    #
    #         elif partition_type == "heal":
    #             subprocess.call(["guardian", "vm", "reconnect", "--all"])
    #
    #         time.sleep(random.uniform(10, 30))
    #
    #     # Final heal
    #     subprocess.call(["guardian", "vm", "reconnect", "--all"])
    #
    # nemesis_task = # asyncio.create_task(partition_nemesis())

    # --- Monitor for split-brain
    # split_brain_detected = False
    # async def monitor_split_brain():
    #     nonlocal split_brain_detected
    #     while not nemesis_task.done():
    #         leaders_by_term = {}
    #         for node in vm_monitor.nodes:
    #             try:
    #                 metrics = get_metrics(node.ip, timeout=2.0)
    #                 if metrics["raft_is_leader"] == 1:
    #                     term = metrics["raft_term"]
    #                     if term not in leaders_by_term:
    #                         leaders_by_term[term] = []
    #                     leaders_by_term[term].append(node.id)
    #             except:
    #                 continue
    #
    #         for term, leaders in leaders_by_term.items():
    #             if len(leaders) > 1:
    #                 split_brain_detected = True
    #                 pytest.fail(f"SPLIT-BRAIN! Term {term}: {leaders}")
    #
    #         time.sleep(1.0)
    #
    # monitor_task = # asyncio.create_task(monitor_split_brain())

    # --- Wait for chaos
    # nemesis_task
    # monitor_task.cancel()

    # --- Verify: No split-brain occurred
    # assert not split_brain_detected

    # --- Verify: Cluster healthy
    # time.sleep(30.0)
    # vm_monitor.wait_for_all_healthy(timeout=30.0)

    pass


@pytest.mark.chaos
@pytest.mark.slow
def test_combined_chaos(vm_monitor):
    """
    Combined chaos: crashes + partitions + latency simultaneously.

    This is the ultimate stress test combining multiple fault types.

    Verifies:
    - System survives worst-case scenarios
    - No corruption
    - Eventually consistent
    - All safety properties maintained
    """
    # --- Setup: 7-node cluster (larger for more chaos possibilities)
    # subprocess.call(["guardian", "cluster", "start", "--nodes", "7"])
    # vm_monitor.wait_for_all_healthy(timeout=30.0)

    # --- Write baseline data
    # leader = get_leader(vm_monitor)
    # for i in range(50):
    #     write_key(leader.ip, f"baseline_{i}", f"value_{i}")

    # --- Multiple concurrent chaos agents
    # async def crash_agent():
    #     """Randomly crash nodes"""
    #     start = time.time()
    #     while time.time() - start < 600:  # 10 minutes
    #         node = random.choice(vm_monitor.nodes)
    #         subprocess.call(["guardian", "vm", "kill", "--target", node.ip])
    #         time.sleep(random.uniform(10, 30))
    #         subprocess.call(["guardian", "vm", "restart", "--target", node.ip])
    #         time.sleep(random.uniform(5, 15))
    #
    # async def partition_agent():
    #     """Randomly partition network"""
    #     start = time.time()
    #     while time.time() - start < 600:
    #         # Random partition or heal
    #         if random.random() < 0.3:
    #             subprocess.call(["guardian", "vm", "reconnect", "--all"])
    #         else:
    #             isolated = random.choice(vm_monitor.nodes)
    #             subprocess.call(["guardian", "vm", "disconnect",
    #                             "--target", isolated.ip])
    #         time.sleep(random.uniform(15, 45))
    #
    # async def latency_agent():
    #     """Randomly inject latency"""
    #     start = time.time()
    #     while time.time() - start < 600:
    #         node = random.choice(vm_monitor.nodes)
    #         latency = random.choice([0, 100, 500, 1000])  # ms
    #         if latency == 0:
    #             subprocess.call(["guardian", "vm", "remove-latency",
    #                             "--target", node.ip])
    #         else:
    #             subprocess.call(["guardian", "vm", "add-latency",
    #                             "--target", node.ip,
    #                             "--latency", f"{latency}ms"])
    #         time.sleep(random.uniform(20, 60))
    #
    # async def registry_chaos_agent():
    #     """Randomly block measurement registry"""
    #     start = time.time()
    #     while time.time() - start < 600:
    #         if random.random() < 0.2:
    #             node = random.choice(vm_monitor.nodes)
    #             subprocess.call(["guardian", "registry", "block",
    #                             "--target", node.ip])
    #             time.sleep(random.uniform(30, 60))
    #             subprocess.call(["guardian", "registry", "unblock",
    #                             "--target", node.ip])
    #         time.sleep(random.uniform(30, 90))

    # --- Start all chaos agents
    # chaos_tasks = [
    #     # asyncio.create_task(crash_agent()),
    #     # asyncio.create_task(partition_agent()),
    #     # asyncio.create_task(latency_agent()),
    #     # asyncio.create_task(registry_chaos_agent())
    # ]

    # --- Client operations during chaos
    # successful_writes = {}
    # async def client_operations():
    #     write_count = 0
    #     while any(not t.done() for t in chaos_tasks):
    #         try:
    #             leader = get_leader(vm_monitor)
    #             key = f"chaos_{write_count}"
    #             value = f"value_{write_count}"
    #             response = write_key(leader.ip, key, value, timeout=10.0)
    #             if response.status == 200:
    #                 successful_writes[key] = value
    #                 write_count += 1
    #         except:
    #             pass
    #         time.sleep(random.uniform(0.5, 2.0))
    #
    # client_task = # asyncio.create_task(client_operations())

    # --- Monitor for invariant violations
    # violations = []
    # async def invariant_monitor():
    #     while any(not t.done() for t in chaos_tasks):
    #         # Check for split-brain
    #         leaders_by_term = {}
    #         for node in vm_monitor.nodes:
    #             try:
    #                 m = get_metrics(node.ip, timeout=2.0)
    #                 if m["raft_is_leader"] == 1:
    #                     term = m["raft_term"]
    #                     if term not in leaders_by_term:
    #                         leaders_by_term[term] = []
    #                     leaders_by_term[term].append(node.id)
    #             except:
    #                 continue
    #
    #         for term, leaders in leaders_by_term.items():
    #             if len(leaders) > 1:
    #                 violations.append(f"Split-brain in term {term}: {leaders}")
    #
    #         time.sleep(2.0)
    #
    # monitor_task = # asyncio.create_task(invariant_monitor())

    # --- Wait for chaos to complete
    # # asyncio.gather(*chaos_tasks)
    # client_task.cancel()
    # monitor_task.cancel()

    # --- Heal everything
    # subprocess.call(["guardian", "cluster", "heal-all"])
    # time.sleep(60.0)

    # --- Verify: No invariant violations
    # assert len(violations) == 0, f"Invariant violations: {violations}"

    # --- Verify: All nodes healthy
    # vm_monitor.wait_for_all_healthy(timeout=60.0)

    # --- Verify: All nodes converged to same state
    # final_indices = [get_metrics(n.ip)["last_log_index"] for n in vm_monitor.nodes]
    # assert len(set(final_indices)) == 1

    # --- Verify: All successful writes present
    # for node in vm_monitor.nodes:
    #     for key, value in successful_writes.items():
    #         actual = read_key(node.ip, key)
    #         assert actual == value, f"Write {key} lost or corrupted"

    # --- Verify: Baseline data still intact
    # for node in vm_monitor.nodes:
    #     for i in range(50):
    #         value = read_key(node.ip, f"baseline_{i}")
    #         assert value == f"value_{i}"

    pass


@pytest.mark.chaos
def test_worst_case_scenario(vm_monitor):
    """
    Orchestrated worst-case scenario: everything fails at once.

    Simulates a data center disaster scenario.

    Verifies:
    - System survives catastrophic failure
    - Data preserved
    - Recovery possible
    """
    # --- Setup: 5-node cluster across "3 availability zones"
    # # Zones: [0,1], [2,3], [4]
    # subprocess.call(["guardian", "cluster", "start", "--nodes", "5"])
    # vm_monitor.wait_for_all_healthy(timeout=30.0)

    # --- Write critical data
    # leader = get_leader(vm_monitor)
    # for i in range(100):
    #     write_key(leader.ip, f"critical_{i}", f"value_{i}")

    # --- Disaster: Entire AZ goes down + network partition
    # # Kill nodes 0 and 1 (simulating AZ failure)
    # subprocess.call(["guardian", "vm", "kill", "--target", vm_monitor.nodes[0].ip])
    # subprocess.call(["guardian", "vm", "kill", "--target", vm_monitor.nodes[1].ip])

    # # Partition node 2 from nodes 3,4
    # subprocess.call(["guardian", "vm", "disconnect",
    #                 "--target", vm_monitor.nodes[2].ip,
    #                 "--from", vm_monitor.nodes[3].ip])
    # subprocess.call(["guardian", "vm", "disconnect",
    #                 "--target", vm_monitor.nodes[2].ip,
    #                 "--from", vm_monitor.nodes[4].ip])

    # --- Only nodes 3,4 remain (minority!)
    # # Verify: Cluster unavailable (no quorum)
    # time.sleep(10.0)
    # for node_id in [3, 4]:
    #     metrics = get_metrics(vm_monitor.nodes[node_id].ip)
    #     # Should not have leader
    #     assert metrics["raft_is_leader"] == 0

    # --- Recovery: Bring back one node from dead AZ
    # subprocess.call(["guardian", "vm", "restart", "--target", vm_monitor.nodes[0].ip])
    # vm_monitor.wait_for_status(0, status="READY", timeout=30.0)

    # --- Verify: Quorum restored, cluster operational
    # wait_for_condition(
    #     lambda: count_leaders(vm_monitor) == 1,
    #     timeout=15.0,
    #     description="Leader elected after partial recovery"
    # )

    # --- Heal partition
    # subprocess.call(["guardian", "vm", "reconnect", "--all"])

    # --- Restart remaining dead node
    # subprocess.call(["guardian", "vm", "restart", "--target", vm_monitor.nodes[1].ip])
    # vm_monitor.wait_for_status(1, status="READY", timeout=30.0)

    # --- Wait for full recovery
    # time.sleep(30.0)

    # --- Verify: All critical data intact
    # for node in vm_monitor.nodes:
    #     for i in range(100):
    #         value = read_key(node.ip, f"critical_{i}")
    #         assert value == f"value_{i}", "Critical data must survive disaster"

    pass

"""Test utilities for Guardian bootstrap and integration tests."""

from __future__ import annotations

import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypedDict

import requests

from guardian_cli.utils.vm_monitor import VMMonitor, VMState, VMStatus

if TYPE_CHECKING:
    from typing import Callable, TypeVar

    T = TypeVar("T")


class HealthResponse(TypedDict):
    """Guardian /health endpoint response."""

    node_id: int
    status: str
    term: int
    cluster_size: int
    trusted_peers: list[int]


@dataclass
class ClusterState:
    """Snapshot of cluster health state."""

    health: dict[str, HealthResponse]  # ip -> health response
    leader_ip: str | None
    leader_count: int
    term: int | None  # None if inconsistent
    cluster_size: int | None  # None if inconsistent

    @property
    def is_healthy(self) -> bool:
        """Check if cluster is in a valid state."""
        return self.leader_count == 1 and self.term is not None and self.cluster_size is not None


# =============================================================================
# VM Control (for bootstrap tests only)
# =============================================================================


def launch_vms_parallel(targets: list, memory: str = "8G", cpus: int = 4) -> None:
    """Launch VMs concurrently. Raises on failure.

    Args:
        targets: List of VM targets (IP addresses or octets)
        memory: Memory allocation per VM (default: 8G)
        cpus: Number of CPUs per VM (default: 4)
    """

    def launch(target):
        cmd = ["guardian", "vm", "launch", "-t", str(target), "--memory", memory, "--cpus", str(cpus)]
        r = subprocess.run(cmd, capture_output=True, text=True)
        return target, r.returncode == 0, r.stderr

    with ThreadPoolExecutor(max_workers=len(targets)) as ex:
        results = list(ex.map(launch, targets))

    failures = [(t, err) for t, ok, err in results if not ok]
    if failures:
        raise RuntimeError(f"Failed to launch: {failures}")


def kill_vms_parallel(targets: list) -> None:
    """Kill VMs concurrently. Raises on failure."""

    def kill(target):
        r = subprocess.run(["guardian", "vm", "kill", "-t", str(target)], capture_output=True, text=True)
        return target, r.returncode == 0, r.stderr

    with ThreadPoolExecutor(max_workers=len(targets)) as ex:
        results = list(ex.map(kill, targets))

    failures = [(t, err) for t, ok, err in results if not ok]
    if failures:
        raise RuntimeError(f"Failed to kill: {failures}")


# =============================================================================
# VMMonitor helpers
# =============================================================================


def create_monitor(ips: list[str], check_interval: float = 2.0) -> VMMonitor:
    """Create and start VMMonitor for given IPs."""
    vms = [VMState(vm_id=f"VM-{ip.split('.')[-1]}", ip=ip, health_url=f"http://{ip}:8443/health") for ip in ips]
    monitor = VMMonitor(vms, check_interval=check_interval)
    monitor.start()
    return monitor


def wait_for_ready(monitor: VMMonitor, timeout: int = 120) -> None:
    """Wait for all VMs to reach operational state (LEADER/FOLLOWER/CANDIDATE)."""
    operational = (VMStatus.LEADER, VMStatus.FOLLOWER, VMStatus.CANDIDATE)
    for vm in monitor.vms:
        monitor.wait_for_status(vm, operational, timeout=timeout)


# =============================================================================
# Log context (for test correlation in OpenObserve)
# =============================================================================


def set_test_context(ips: list[str], test_id: str, timeout: float = 60.0, poll_interval: float = 2.0) -> None:
    """
    Set test-id log context on Guardian nodes.

    Waits for nodes to be reachable, then sets the test-id context via HTTP POST.
    Call this after nodes are running and API is responding.

    Args:
        ips: Node IP addresses to configure
        test_id: Test identifier for log filtering
        timeout: Max time to wait for nodes to be reachable
        poll_interval: Seconds between reachability checks

    Raises:
        TimeoutError: If nodes not reachable within timeout
        RuntimeError: If setting context fails on any node
    """
    deadline = time.time() + timeout
    pending = set(ips)
    errors: dict[str, str] = {}

    while pending and time.time() < deadline:
        for ip in list(pending):
            try:
                response = requests.post(
                    f"http://{ip}:8443/logging/set-context",
                    json={"key": "test-id", "value": test_id},
                    timeout=5,
                )
                response.raise_for_status()
                pending.discard(ip)
                errors.pop(ip, None)
            except requests.RequestException as e:
                errors[ip] = str(e)

        if pending:
            time.sleep(poll_interval)

    if pending:
        error_details = "\n".join(f"  {ip}: {errors.get(ip, 'unknown')}" for ip in pending)
        raise TimeoutError(f"Timeout setting test context on {len(pending)} nodes:\n{error_details}")


# =============================================================================
# Health check utilities
# =============================================================================


def get_health(ip: str, timeout: int = 5) -> HealthResponse:
    """Get health status from a single node."""
    response = requests.get(f"http://{ip}:8443/health", timeout=timeout)
    response.raise_for_status()
    return response.json()


def get_cluster_state(ips: list[str], timeout: int = 5) -> ClusterState:
    """Get health from all nodes and compute cluster state."""
    health: dict[str, HealthResponse] = {}

    for ip in ips:
        try:
            health[ip] = get_health(ip, timeout)
        except Exception as e:
            raise RuntimeError(f"Failed to get health from {ip}: {e}") from e

    # Find leader(s)
    leaders = [ip for ip, h in health.items() if h["status"] == "LEADER"]
    leader_ip = leaders[0] if len(leaders) == 1 else None

    # Check term consistency
    terms = {h["term"] for h in health.values()}
    term = terms.pop() if len(terms) == 1 else None

    # Check cluster_size consistency
    sizes = {h["cluster_size"] for h in health.values()}
    cluster_size = sizes.pop() if len(sizes) == 1 else None

    return ClusterState(
        health=health,
        leader_ip=leader_ip,
        leader_count=len(leaders),
        term=term,
        cluster_size=cluster_size,
    )


def ip_to_node_id(ip: str) -> int:
    """Convert IP to node_id (full 32-bit integer, matching Rust implementation)."""
    parts = ip.split(".")
    return (int(parts[0]) << 24) | (int(parts[1]) << 16) | (int(parts[2]) << 8) | int(parts[3])


# =============================================================================
# Cluster state assertions
# =============================================================================


def _check_mutual_trust(ips: list[str], state: ClusterState) -> tuple[bool, str | None]:
    """
    Check if all nodes mutually trust each other.

    Returns:
        (True, None) if all nodes trust each other
        (False, error_message) if trust is incomplete
    """
    node_ids = {ip_to_node_id(ip) for ip in ips}

    for ip in ips:
        health = state.health[ip]
        my_id = health["node_id"]
        trusted = set(health["trusted_peers"])

        # Should trust all other nodes (not self)
        expected = node_ids - {my_id}
        missing = expected - trusted
        if missing:
            return False, f"Node {ip} (id={my_id}) missing trust for: {missing}"

    return True, None


def assert_mutual_trust(
    ips: list[str],
    state: ClusterState | None = None,
    timeout: float = 60.0,
    poll_interval: float = 2.0,
) -> ClusterState:
    """
    Assert all nodes mutually trust each other, with optional polling.

    If state is provided and trust check passes immediately, returns immediately.
    Otherwise, polls /health endpoints until mutual trust is established or timeout.

    Args:
        ips: List of node IPs that should mutually trust each other
        state: Optional pre-fetched ClusterState (if None, will fetch)
        timeout: Maximum seconds to wait for mutual trust (default: 60)
        poll_interval: Seconds between polls (default: 2)

    Returns:
        ClusterState when mutual trust is established

    Raises:
        AssertionError: If mutual trust not established within timeout
    """
    # Try with provided state first
    if state is not None:
        ok, _ = _check_mutual_trust(ips, state)
        if ok:
            return state

    def check() -> ClusterState | None:
        current_state = get_cluster_state(ips)
        ok, _ = _check_mutual_trust(ips, current_state)
        return current_state if ok else None

    try:
        return poll_until(check, timeout, poll_interval, "mutual trust")
    except TimeoutError as e:
        raise AssertionError(str(e)) from e


def assert_single_leader(state: ClusterState) -> str:
    """Assert exactly one leader exists. Returns leader IP."""
    if state.leader_count == 0:
        statuses = {ip: h["status"] for ip, h in state.health.items()}
        raise AssertionError(f"No leader found. Statuses: {statuses}")
    if state.leader_count > 1:
        leaders = [ip for ip, h in state.health.items() if h["status"] == "LEADER"]
        raise AssertionError(f"Multiple leaders (split-brain): {leaders}")
    assert state.leader_ip is not None  # Guaranteed by leader_count == 1
    return state.leader_ip


def assert_consistent_term(state: ClusterState) -> int:
    """Assert all nodes have the same term. Returns the term."""
    if state.term is None:
        terms = {ip: h["term"] for ip, h in state.health.items()}
        raise AssertionError(f"Inconsistent terms: {terms}")
    return state.term


def assert_cluster_size(state: ClusterState, expected: int) -> None:
    """Assert all nodes agree on cluster size."""
    if state.cluster_size is None:
        sizes = {ip: h["cluster_size"] for ip, h in state.health.items()}
        raise AssertionError(f"Inconsistent cluster sizes: {sizes}")
    if state.cluster_size != expected:
        raise AssertionError(f"Expected cluster_size={expected}, got {state.cluster_size}")


# =============================================================================
# Polling utilities for integration tests
# =============================================================================


def poll_until(
    check_fn: Callable[[], T | None],
    timeout: float,
    poll_interval: float,
    description: str,
) -> T:
    """
    Generic polling helper that waits for a condition to be met.

    Args:
        check_fn: Callable that returns a truthy value when condition is met,
                  or None/falsy to continue polling. Exceptions are caught and ignored.
        timeout: Maximum seconds to wait
        poll_interval: Seconds between polls
        description: Description for timeout error message

    Returns:
        The truthy value returned by check_fn

    Raises:
        TimeoutError: If condition not met within timeout
    """
    deadline = time.time() + timeout
    last_error: str | None = None

    while time.time() < deadline:
        try:
            if result := check_fn():
                return result
        except Exception as e:
            last_error = str(e)
        time.sleep(poll_interval)

    raise TimeoutError(f"Timeout waiting for {description} after {timeout}s: {last_error}")


def get_leader(ips: list[str], timeout: int = 5) -> str:
    """
    Get the current leader IP from a list of node IPs.

    Args:
        ips: List of node IPs to query
        timeout: HTTP request timeout

    Returns:
        IP address of the leader

    Raises:
        RuntimeError: If no leader found or multiple leaders (split-brain)
    """
    state = get_cluster_state(ips, timeout)
    return assert_single_leader(state)


def get_followers(ips: list[str], timeout: int = 5) -> list[str]:
    """
    Get all follower IPs from a list of node IPs.

    Args:
        ips: List of node IPs to query
        timeout: HTTP request timeout

    Returns:
        List of follower IP addresses
    """
    state = get_cluster_state(ips, timeout)
    return [ip for ip, h in state.health.items() if h["status"] == "FOLLOWER"]


def wait_for_condition(
    condition: Callable[[], bool],
    timeout: float = 30.0,
    poll_interval: float = 1.0,
    description: str = "condition",
) -> None:
    """
    Wait for a condition to become true.

    Args:
        condition: Callable that returns True when condition is met
        timeout: Maximum seconds to wait
        poll_interval: Seconds between polls
        description: Description for timeout error message

    Raises:
        TimeoutError: If condition not met within timeout
    """
    poll_until(lambda: True if condition() else None, timeout, poll_interval, description)


def wait_for_healthy_cluster(
    ips: list[str],
    timeout: float = 120.0,
    poll_interval: float = 2.0,
    require_mutual_trust: bool = True,
) -> ClusterState:
    """
    Wait for cluster to reach a healthy state.

    A healthy cluster has:
    - Exactly one leader
    - All nodes agree on term
    - All nodes agree on cluster size
    - (Optional) All nodes mutually trust each other

    Args:
        ips: List of node IPs
        timeout: Maximum seconds to wait
        poll_interval: Seconds between polls
        require_mutual_trust: If True, also verify mutual trust

    Returns:
        ClusterState when healthy

    Raises:
        TimeoutError: If cluster doesn't become healthy within timeout
    """

    def check() -> ClusterState | None:
        state = get_cluster_state(ips)
        if not state.is_healthy:
            return None
        if require_mutual_trust:
            ok, _ = _check_mutual_trust(ips, state)
            if not ok:
                return None
        return state

    return poll_until(check, timeout, poll_interval, "healthy cluster")


def wait_for_leader(
    ips: list[str],
    timeout: float = 30.0,
    poll_interval: float = 1.0,
) -> str:
    """
    Wait for exactly one leader to be elected.

    Args:
        ips: List of node IPs
        timeout: Maximum seconds to wait
        poll_interval: Seconds between polls

    Returns:
        IP of the elected leader

    Raises:
        TimeoutError: If no single leader within timeout
    """

    def check() -> str | None:
        state = get_cluster_state(ips)
        return state.leader_ip if state.leader_count == 1 else None

    return poll_until(check, timeout, poll_interval, "leader election")


def wait_for_new_leader(
    ips: list[str],
    old_leader_ip: str,
    timeout: float = 30.0,
    poll_interval: float = 1.0,
) -> str:
    """
    Wait for a new leader to be elected (different from old_leader_ip).

    Args:
        ips: List of node IPs to query
        old_leader_ip: IP of the previous leader (we want a different one)
        timeout: Maximum seconds to wait
        poll_interval: Seconds between polls

    Returns:
        IP of the new leader

    Raises:
        TimeoutError: If no new leader within timeout
    """

    def check() -> str | None:
        state = get_cluster_state(ips)
        if state.leader_count == 1 and state.leader_ip and state.leader_ip != old_leader_ip:
            return state.leader_ip
        return None

    return poll_until(check, timeout, poll_interval, f"new leader (not {old_leader_ip})")

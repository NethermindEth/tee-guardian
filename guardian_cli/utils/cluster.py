"""
Cluster allocation with flock-based locking for test parallelization.

Manages exclusive access to test clusters on the TDX host using:
- flock() for atomic acquisition
- mtime-based TTL for automatic decay of stale locks
- Background heartbeat to keep locks alive during long test runs

Cluster mapping (Nebula IPs via overlay network):
    Cluster 0: 10.42.1.20-29 (TDX host)
    Cluster 1: 10.42.1.30-39
    ...
    Cluster 9: 10.42.1.110-119

Docker services (Nebula IPs, routed via host running them):
    measurement-registry: 10.42.100.2
    openobserve: 10.42.100.100
"""

from __future__ import annotations

import getpass
import json
import logging
import platform
import threading
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from typing import Callable

logger = logging.getLogger("guardian.cluster")

# Constants
LOCK_DIR = "/var/run/guardian-locks"
TTL_SECONDS = 120  # Lock expires after 2 minutes without heartbeat
HEARTBEAT_INTERVAL = 60  # Touch lock file every 60 seconds
NUM_CLUSTERS = 10
DEFAULT_HOST_ID = 1  # TDX host

# Docker services via Nebula (routed via the host running them)
DOCKER_SUBNET = "10.42.100.0/24"
REGISTRY_IP = "10.42.100.2"
OPENOBSERVE_IP = "10.42.100.100"


@dataclass
class ClusterAllocation:
    """
    Represents an allocated cluster for testing.

    Attributes:
        cluster_id: Cluster identifier (0-9)
        ips: List of 10 IP addresses for this cluster
        session_id: Unique session identifier
        lock_path: Path to the lock file on the TDX host
        acquired_at: When the lock was acquired
    """

    cluster_id: int
    ips: list[str]
    session_id: str
    lock_path: str
    acquired_at: datetime = field(default_factory=datetime.utcnow)


def cluster_id_to_ips(cluster_id: int, host_types: list[str] | None = None) -> dict[str, list[str]]:
    """
    Convert a cluster ID to its Nebula IP ranges across hosts.

    Cluster 0: tdx=10.42.1.20-29, sev=10.42.2.20-29
    Cluster 1: tdx=10.42.1.30-39, sev=10.42.2.30-39
    ...

    Args:
        cluster_id: Cluster ID (0-9)
        host_types: List of host types to include (default: ["tdx"])

    Returns:
        Dict mapping host_type -> list of 10 IPs
    """
    from . import HOSTS

    if cluster_id < 0 or cluster_id > 9:
        raise ValueError(f"cluster_id must be 0-9, got {cluster_id}")

    if host_types is None:
        host_types = ["tdx"]

    base_octet = 20 + (cluster_id * 10)
    result = {}
    for ht in host_types:
        host_id = HOSTS[ht]["id"]
        result[ht] = [f"10.42.{host_id}.{base_octet + i}" for i in range(10)]
    return result


def registry_url() -> str:
    """
    Get the URL of the measurement registry.

    The registry is accessible via Nebula at a fixed IP.

    Returns:
        URL string (e.g., "http://10.42.100.2:9000")
    """
    return f"http://{REGISTRY_IP}:9000"


@dataclass
class LockInfo:
    """Information stored in a lock file."""

    session_id: str
    holder: str
    acquired_at: datetime
    purpose: str
    mtime: float | None = None

    def is_stale(self, ttl_seconds: int = TTL_SECONDS) -> bool:
        """Check if this lock is stale based on mtime."""
        if self.mtime is None:
            return False
        return (time.time() - self.mtime) > ttl_seconds

    def age_seconds(self) -> float:
        """Seconds since last heartbeat."""
        if self.mtime is None:
            return 0
        return time.time() - self.mtime

    def to_json(self) -> str:
        """Serialize to JSON for writing to lock file."""
        return json.dumps(
            {
                "session_id": self.session_id,
                "holder": self.holder,
                "acquired_at": self.acquired_at.isoformat(),
                "purpose": self.purpose,
            }
        )

    @classmethod
    def from_json(cls, data: str, mtime: float) -> LockInfo:
        """Deserialize from JSON read from lock file."""
        parsed = json.loads(data)
        return cls(
            session_id=parsed["session_id"],
            holder=parsed["holder"],
            acquired_at=datetime.fromisoformat(parsed["acquired_at"]),
            purpose=parsed["purpose"],
            mtime=mtime,
        )


class ClusterAllocator:
    """
    Manages cluster allocation with distributed locking.

    Uses flock-based locks on the TDX host with mtime-based decay.
    A background thread sends heartbeats to keep locks alive.

    Usage:
        allocator = ClusterAllocator(config)
        try:
            cluster = allocator.acquire()
            # ... use cluster.ips ...
        finally:
            allocator.release(cluster)

    Or as context manager:
        with ClusterAllocator(config) as allocator:
            cluster = allocator.acquire()
            # ... automatically released on exit ...
    """

    def __init__(self, config: dict):
        """
        Initialize allocator with Guardian config.

        Args:
            config: Guardian configuration dict (from load_config())
        """
        self.config = config
        self._allocations: dict[int, ClusterAllocation] = {}
        self._heartbeat_threads: dict[int, threading.Thread] = {}
        self._running = True
        self._lock = threading.Lock()

    def _ssh_execute(self, command: str, raise_on_error: bool = True) -> tuple[int, str, str]:
        """Execute command on TDX host via SSH (locks are shared across all hosts)."""
        from . import remote_execute, SSHResult

        # Always use TDX host for cluster locks (it's the primary lighthouse)
        result = remote_execute(self.config, command, streaming=False, raise_on_nonzero=False, host_type="tdx")
        assert isinstance(result, SSHResult)

        if raise_on_error and result.returncode != 0:
            raise RuntimeError(f"SSH command failed: {command}\nstderr: {result.stderr}")

        return result.returncode, result.stdout, result.stderr

    def _ensure_lock_dir(self) -> None:
        """Ensure lock directory exists on TDX host."""
        self._ssh_execute(f"mkdir -p {LOCK_DIR}", raise_on_error=True)

    def _read_lock(self, cluster_id: int) -> LockInfo | None:
        """Read lock file for a cluster."""
        lock_path = f"{LOCK_DIR}/cluster-{cluster_id}.lock"

        cmd = f"""
        if [ -f {lock_path} ]; then
            stat -c '%Y' {lock_path}
            cat {lock_path}
        else
            echo "NOLOCK"
        fi
        """
        returncode, stdout, _ = self._ssh_execute(cmd, raise_on_error=False)

        if returncode != 0 or stdout.strip() == "NOLOCK":
            return None

        lines = stdout.strip().split("\n", 1)
        if len(lines) < 2:
            return None

        try:
            mtime = float(lines[0])
            lock_data = lines[1]
            return LockInfo.from_json(lock_data, mtime)
        except (ValueError, json.JSONDecodeError) as e:
            logger.warning(f"Corrupted lock file for cluster-{cluster_id}: {e}")
            return None

    def _try_acquire_lock(self, cluster_id: int, session_id: str, purpose: str) -> bool:
        """Try to acquire lock for a cluster using flock."""
        lock_path = f"{LOCK_DIR}/cluster-{cluster_id}.lock"
        holder = f"{getpass.getuser()}@{platform.node()}"

        lock_info = LockInfo(
            session_id=session_id,
            holder=holder,
            acquired_at=datetime.utcnow(),
            purpose=purpose,
        )
        lock_json = lock_info.to_json()

        # Use flock with non-blocking mode (-n)
        cmd = f"""
        exec 200>{lock_path}
        if flock -n 200; then
            echo '{lock_json}' > {lock_path}
            exit 0
        else
            exit 1
        fi
        """
        returncode, _, _ = self._ssh_execute(cmd, raise_on_error=False)
        return returncode == 0

    def _release_lock(self, cluster_id: int) -> None:
        """Release lock for a cluster."""
        lock_path = f"{LOCK_DIR}/cluster-{cluster_id}.lock"
        self._ssh_execute(f"rm -f {lock_path}", raise_on_error=False)

    def _heartbeat_thread(self, cluster_id: int) -> None:
        """Background thread that touches lock file to keep it alive."""
        lock_path = f"{LOCK_DIR}/cluster-{cluster_id}.lock"

        while self._running:
            with self._lock:
                if cluster_id not in self._allocations:
                    break

            try:
                self._ssh_execute(f"touch {lock_path}", raise_on_error=False)
            except Exception as e:
                logger.warning(f"Heartbeat failed for cluster-{cluster_id}: {e}")

            # Sleep in small intervals so we can exit quickly
            for _ in range(HEARTBEAT_INTERVAL):
                if not self._running:
                    break
                with self._lock:
                    if cluster_id not in self._allocations:
                        break
                time.sleep(1)

    def list_clusters(self) -> list[tuple[int, LockInfo | None]]:
        """List all clusters and their lock status."""
        self._ensure_lock_dir()
        result = []

        for cluster_id in range(NUM_CLUSTERS):
            lock_info = self._read_lock(cluster_id)
            result.append((cluster_id, lock_info))

        return result

    def find_available(self) -> list[int]:
        """Find all available cluster IDs (free or stale)."""
        available = []

        for cluster_id, lock_info in self.list_clusters():
            if lock_info is None:
                available.append(cluster_id)
            elif lock_info.is_stale():
                available.append(cluster_id)

        return available

    def acquire(
        self,
        preferred_id: int | None = None,
        purpose: str = "test",
    ) -> ClusterAllocation:
        """
        Acquire a cluster for testing.

        Args:
            preferred_id: Preferred cluster ID (will try this first)
            purpose: Description of what this cluster is for

        Returns:
            ClusterAllocation with cluster details

        Raises:
            RuntimeError: If no clusters available
        """
        self._ensure_lock_dir()
        session_id = f"{getpass.getuser()}-{uuid.uuid4().hex[:8]}"

        # Build list of clusters to try
        clusters_to_try = []
        if preferred_id is not None and 0 <= preferred_id < NUM_CLUSTERS:
            clusters_to_try.append(preferred_id)

        for i in range(NUM_CLUSTERS):
            if i not in clusters_to_try:
                clusters_to_try.append(i)

        # Try each cluster
        for cluster_id in clusters_to_try:
            lock_info = self._read_lock(cluster_id)

            if lock_info is not None:
                if lock_info.is_stale():
                    logger.warning(
                        f"Taking over stale lock for cluster-{cluster_id} "
                        f"from {lock_info.holder} (last heartbeat {lock_info.age_seconds():.0f}s ago)"
                    )
                    self._release_lock(cluster_id)
                else:
                    continue

            if self._try_acquire_lock(cluster_id, session_id, purpose):
                # Get TDX IPs for the allocation (cluster allocator is TDX-focused)
                ips_by_host = cluster_id_to_ips(cluster_id, host_types=["tdx"])
                allocation = ClusterAllocation(
                    cluster_id=cluster_id,
                    ips=ips_by_host["tdx"],
                    session_id=session_id,
                    lock_path=f"{LOCK_DIR}/cluster-{cluster_id}.lock",
                )

                with self._lock:
                    self._allocations[cluster_id] = allocation

                thread = threading.Thread(
                    target=self._heartbeat_thread,
                    args=(cluster_id,),
                    daemon=True,
                    name=f"heartbeat-cluster-{cluster_id}",
                )
                self._heartbeat_threads[cluster_id] = thread
                thread.start()

                logger.info(
                    f"Acquired cluster-{cluster_id} "
                    f"(IPs: {allocation.ips[0]}-{allocation.ips[-1].split('.')[-1]}, "
                    f"session: {session_id})"
                )
                return allocation

        # No clusters available
        cluster_status = []
        for cluster_id, lock_info in self.list_clusters():
            if lock_info:
                cluster_status.append(
                    f"  cluster-{cluster_id}: {lock_info.holder} ({lock_info.age_seconds():.0f}s ago)"
                )
            else:
                cluster_status.append(f"  cluster-{cluster_id}: FREE")

        raise RuntimeError(
            f"No clusters available. All {NUM_CLUSTERS} clusters are in use:\n" + "\n".join(cluster_status)
        )

    def release(self, allocation: ClusterAllocation) -> None:
        """Release a cluster allocation."""
        cluster_id = allocation.cluster_id

        with self._lock:
            if cluster_id in self._allocations:
                del self._allocations[cluster_id]

        if cluster_id in self._heartbeat_threads:
            del self._heartbeat_threads[cluster_id]

        self._release_lock(cluster_id)
        logger.info(f"Released cluster-{cluster_id} (session: {allocation.session_id})")

    def release_all(self) -> None:
        """Release all allocations held by this allocator."""
        self._running = False

        with self._lock:
            allocations = list(self._allocations.values())

        for allocation in allocations:
            self.release(allocation)

    def __enter__(self) -> ClusterAllocator:
        return self

    def __exit__(self, *args) -> None:
        self.release_all()


def clear_cluster_registry(config: dict, cluster_id: int) -> None:
    """
    Clear a cluster's registry configuration.

    Uses the Nebula IP to call the registry's DELETE endpoint.
    Failures are logged but not raised (cleanup is best-effort).

    Args:
        config: Guardian config (unused, kept for API compatibility)
        cluster_id: Cluster ID (0-9)
    """
    import requests

    url = f"{registry_url()}/config"

    try:
        response = requests.delete(url, timeout=10)
        response.raise_for_status()
        logger.info(f"Cleared registry for cluster-{cluster_id}")
    except Exception as e:
        logger.warning(f"Failed to clear registry for cluster-{cluster_id}: {e}")

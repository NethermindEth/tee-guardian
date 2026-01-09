"""
PyTest fixtures for Guardian integration tests.

Integration tests operate on an already-running cluster. They use network
isolation (disconnect/reconnect) to test behavior, not VM lifecycle changes.

Key fixtures:
- setup_log_context: Auto-use fixture that sets test-id log context on all nodes
- cluster_logs: Log streaming for behavioral verification

Uses shared fixtures from tests/conftest.py:
- cluster_ips: IP addresses allocated to this worker
"""

from __future__ import annotations

from typing import Generator

import pytest

from tests.helpers import set_test_context
from tests.utils.cluster_logs import ClusterLogs


@pytest.fixture(scope="function", autouse=True)
def setup_log_context(request, cluster_ips, cluster_logs: ClusterLogs):
    """
    Auto-use fixture that sets log context on cluster nodes before each test.

    Integration tests assume the cluster is already running, so this runs
    immediately at test start to ensure logs are correlated by test-id.

    This fixture has priority by being listed first in conftest.py and using
    autouse=True. It depends on cluster_logs to get the test_id.
    """
    test_ips = cluster_ips[:5]
    set_test_context(test_ips, cluster_logs.test_id, timeout=60)
    yield


@pytest.fixture(scope="function")
def cluster_logs(request, cluster_ips) -> Generator[ClusterLogs, None, None]:
    """
    Provides log streaming for cluster integration tests.

    - Automatically starts streaming filtered by test name (test-id)
    - Fails immediately if OpenObserve is unavailable
    - Provides wait_for_pattern() for behavioral verification

    Usage:
        def test_leader_election(cluster_ips, cluster_logs):
            # Log context is set automatically by setup_log_context fixture

            # Wait for all nodes to log a pattern
            cluster_logs.wait_for_pattern(
                r"Cluster bootstrap successful|Joined cluster",
                match_all=[{"host_id": ip} for ip in cluster_ips[:5]],
                timeout=120
            )

            # Check for unexpected errors
            errors = cluster_logs.get_logs(level="ERROR")
            assert not errors, f"Unexpected errors: {errors}"
    """
    test_name = request.node.name

    logs = ClusterLogs(
        test_id=test_name,
        cluster_ips=cluster_ips,
    )
    logs.start()  # Raises ConnectionError if OpenObserve unavailable

    yield logs

    logs.stop()

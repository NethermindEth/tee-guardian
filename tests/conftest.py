"""Shared pytest fixtures for Guardian tests.

Cluster allocation is handled by the guardian CLI before pytest runs.
Tests receive cluster IDs via --cluster-ids parameter and use the
corresponding IP ranges.

Cluster mapping (Nebula IPs via overlay network):
    Cluster 0: 10.42.X.20-29
    Cluster 1: 10.42.X.30-39
    ...
    Cluster 9: 10.42.X.110-119

Measurement flow:
    1. Guardian CLI extracts measurements to global cache:
       /tmp/guardian-rtmr-cache/{boot_img_hash}/measurements.txt
    2. `guardian vm read-measurements` reads from cache based on workspace's boot.img
    3. configure_all_registries fixture configures measurement-registry

Registry:
    measurement-registry: http://10.42.100.2:9000
"""

import re
import subprocess
import sys
from pathlib import Path
from typing import Generator

import pytest
import requests

# Ensure guardian_cli is importable
parent_dir = Path(__file__).parent.parent
if str(parent_dir) not in sys.path:
    sys.path.insert(0, str(parent_dir))

from guardian_cli.utils.cluster import cluster_id_to_ips, REGISTRY_IP


def parse_tdx_info_output(text: str) -> dict[str, str]:
    """Parse tdx-info output to extract RTMR values."""
    measurements = {}
    pattern = r"RTMR\[(\d)\]:\s*([0-9a-fA-F]+)"
    for match in re.finditer(pattern, text):
        index = match.group(1)
        value = match.group(2).lower()
        measurements[f"rtmr{index}"] = value
    if len(measurements) != 4:
        raise ValueError(f"Expected 4 RTMRs, found {len(measurements)}")
    return measurements


def pytest_addoption(parser):
    """Add custom pytest command-line options."""
    parser.addoption(
        "--cluster-ids",
        default="0",
        help="Comma-separated cluster IDs allocated by guardian CLI (e.g., '0' or '0,1,2')",
    )
    parser.addoption(
        "--host-types",
        default="tdx",
        help="Comma-separated host types to run tests on (e.g., 'tdx' or 'tdx,sev')",
    )


def pytest_configure(config):
    """Configure pytest with custom markers."""
    config.addinivalue_line("markers", "integration: Integration tests")
    config.addinivalue_line("markers", "bootstrap: Bootstrap tests")
    config.addinivalue_line("markers", "chaos: Chaos tests")


@pytest.fixture(scope="session")
def cluster_ids(request) -> list[int]:
    """
    List of cluster IDs allocated for this test session.

    Passed via --cluster-ids parameter from the guardian CLI.
    The CLI handles lock acquisition before pytest runs.

    Returns:
        List of cluster IDs (e.g., [0] or [0, 1, 2])
    """
    ids_str = request.config.getoption("--cluster-ids")
    return [int(x.strip()) for x in ids_str.split(",")]


@pytest.fixture(scope="session")
def host_types(request) -> list[str]:
    """
    List of host types to run tests on.

    Passed via --host-types parameter from the guardian CLI.

    Returns:
        List of host types (e.g., ["tdx"] or ["tdx", "sev"])
    """
    types_str = request.config.getoption("--host-types")
    return [x.strip() for x in types_str.split(",")]


@pytest.fixture(scope="session")
def cluster_id(cluster_ids) -> int:
    """
    Primary cluster ID for tests that only need one cluster.

    Returns:
        First cluster ID from the allocated list
    """
    return cluster_ids[0]


@pytest.fixture(scope="session")
def cluster_ips(cluster_id) -> list[str]:
    """
    IP addresses for the primary test cluster (TDX only).

    Returns 10 Nebula IPs based on the cluster_id:
        Cluster 0: 10.42.1.20-29
        Cluster 1: 10.42.1.30-39
        etc.

    Tests can use first 5 for registry nodes, last 5 for gossip nodes.
    """
    ips_by_host = cluster_id_to_ips(cluster_id, host_types=["tdx"])
    return ips_by_host["tdx"]


@pytest.fixture(scope="session")
def all_cluster_ips(cluster_ids) -> dict[int, list[str]]:
    """
    IP addresses for all allocated clusters (TDX only).

    Returns:
        Dict mapping cluster_id -> list of 10 IPs
    """
    return {cid: cluster_id_to_ips(cid, host_types=["tdx"])["tdx"] for cid in cluster_ids}


@pytest.fixture(scope="session")
def measurements() -> dict[str, str]:
    """
    Load TDX measurements from global cache via guardian CLI.

    The CLI computes the boot.img hash and reads from:
        /tmp/guardian-rtmr-cache/{hash}/measurements.txt

    This is workspace-aware - it uses the current workspace's boot.img
    to determine which cached measurements to return.

    Returns:
        Dict with rtmr0, rtmr1, rtmr2, rtmr3 keys
    """
    proc = subprocess.run(
        ["guardian", "vm", "read-measurements"],
        capture_output=True,
        text=True,
        check=True,
    )

    return parse_tdx_info_output(proc.stdout)


@pytest.fixture(scope="session")
def registry_backend(cluster_id) -> str:
    """
    URL of the measurement registry (via Nebula).

    Returns:
        http://10.42.100.2:9000
    """

    return f"http://{REGISTRY_IP}:9000"


@pytest.fixture(scope="session", autouse=True)
def configure_all_registries(cluster_ids, measurements, all_cluster_ips) -> Generator[None, None, None]:
    """
    Configure registry with measurements and bootstrap nodes.

    This is an autouse fixture that runs once per session:
    1. On setup: Configure registry with measurements and bootstrap nodes
    2. On teardown: Clear registry

    All clusters use the same measurements (from the current workspace's boot.img).
    """

    # Configure the registry with the same measurements for all clusters
    for cid in cluster_ids:
        cluster_ips_list = all_cluster_ips[cid]
        bootstrap_ips = cluster_ips_list[:5]  # First 5 IPs are bootstrap nodes

        payload = {
            "namespace": "guardian",
            "measurements": [
                {
                    "rtmr0": measurements["rtmr0"],
                    "rtmr1": measurements["rtmr1"],
                    "rtmr2": measurements["rtmr2"],
                    "rtmr3": measurements["rtmr3"],
                    "revoked": False,
                    "description": f"Cluster {cid} test measurement",
                }
            ],
            "nodes": [{"ip_address": ip, "raft_port": 8444, "api_port": 8443} for ip in bootstrap_ips],
        }

        response = requests.post(f"http://{REGISTRY_IP}:9000/config", json=payload, timeout=10)
        response.raise_for_status()

    yield

    # Cleanup: clear registry
    try:
        requests.delete(f"http://{REGISTRY_IP}:9000/config", timeout=10)
    except Exception:
        pass  # Best effort cleanup

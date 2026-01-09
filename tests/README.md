# Guardian Test Infrastructure

## Architecture

```
Developer Machine                          TDX Host
─────────────────                          ────────
guardian test integration                  
  │                                        
  ├─ Acquire cluster lock ──────────────── /var/run/guardian-locks/cluster-N.lock
  ├─ rsync + nix build ─────────────────── {workspace}/test-vm-result/
  ├─ Extract measurements ──────────────── /tmp/guardian-rtmr-cache/{hash}/
  │                                        
  └─ pytest --cluster-ids=N                
       │                                   
       │  sshuttle VPN                     Docker (172.20.0.0/24)
       │  ─────────────                    ───────────────────────
       ├─ guardian vm read-measurements    measurement-registry (proxy) .2
       ├─ Configure registry ───────────── registry-cluster-N (.10-.19)
       └─ Tests talk to VMs ────────────── VMs on br0 (192.168.122.X0-X9)
```

## Cluster Allocation

10 clusters, 10 VMs each. Locked via flock + mtime heartbeat.

| Cluster | VM IPs              | Registry Backend |
|---------|---------------------|------------------|
| 0       | 192.168.122.20-29   | 172.20.0.10:9000 |
| 1       | 192.168.122.30-39   | 172.20.0.11:9000 |
| ...     | ...                 | ...              |
| 9       | 192.168.122.110-119 | 172.20.0.19:9000 |

## Measurement Extraction

Cached globally by boot.img hash at `/tmp/guardian-rtmr-cache/{sha256}/measurements.txt`.

```bash
# CLI extracts and caches (during test setup)
extract-tdx-measurements --ip-octet 20 /dev/null

# pytest fixtures read via CLI command
guardian vm read-measurements  # Computes hash, reads from cache, prints to stdout
```

Cache hit = instant. Cache miss = boots VM, extracts via `tdx-info`, caches result.

This global cache enables multiple workspaces with the same code to share measurements.

## Test Types

### Integration Tests (`tests/integration/`)

**Purpose**: Test behavior of a running cluster.

- Cluster already formed before test starts
- Tests verify reactions to events (partitions, failures, load)
- VMs launched by CLI, not by tests

```python
@pytest.mark.integration
def test_follower_disconnect(cluster_ips, create_vm_monitor):
    test_ips = cluster_ips[:5]
    # VMs already running from CLI
    monitor = create_vm_monitor(test_ips)
    
    initial_state = get_cluster_state(test_ips)
    disconnect_vms([follower_ip])
    # Assert leader unchanged, term unchanged
    reconnect_vms([follower_ip])
```

### Bootstrap Tests (`tests/bootstrap/`)

**Purpose**: Test cluster formation from scratch.

- Tests launch VMs themselves
- Verify initial leader election, peer discovery, gossip join
- May use multiple clusters for cross-cluster gossip tests

```python
@pytest.mark.bootstrap
def test_cluster_bootstrap(cluster_ips, create_vm_monitor):
    kill_vms(cluster_ips)  # Clean slate
    launch_vms(cluster_ips[:5])
    
    monitor = create_vm_monitor(cluster_ips[:5])
    wait_for_ready(monitor)
    
    state = get_cluster_state(cluster_ips[:5])
    assert_single_leader(state)
    assert_mutual_trust(cluster_ips[:5], state)
```

## Key Fixtures

| Fixture | Scope | Description |
|---------|-------|-------------|
| `cluster_id` | session | Primary cluster ID (first allocated) |
| `cluster_ids` | session | All allocated cluster IDs |
| `cluster_ips` | session | 10 IPs for primary cluster |
| `registry_ips` | session | First 5 IPs (bootstrap nodes) |
| `gossip_ips` | session | Last 5 IPs (gossip-only nodes) |
| `measurements` | session | RTMRs for primary cluster |
| `registry_backend` | session | URL of registry backend |
| `create_vm_monitor` | function | Factory for VMMonitor instances |
| `restore_registry` | function | Restores registry after test modifies it |

`configure_all_registries` is autouse—runs automatically per session.

## Test Helpers

```python
from tests.helpers import (
    launch_vms, kill_vms,           # VM lifecycle
    disconnect_vms, reconnect_vms,  # Network partitions
    get_cluster_state,              # Fetch /health from all nodes
    assert_single_leader,           # Verify exactly one leader
    assert_consistent_term,         # Verify all nodes same term
    assert_mutual_trust,            # Verify all nodes trust each other
    wait_for_ready,                 # Wait for VMs operational
    wait_for_leader,                # Wait for leader election
    wait_for_healthy_cluster,       # Wait for full cluster health
)
```

## Running Tests

```bash
# Integration tests (1 cluster, VMs launched by CLI)
guardian test integration

# Bootstrap tests (1 cluster by default, tests launch VMs)
guardian test bootstrap

# Specific test
guardian test integration -t test_follower_disconnect

# Multiple clusters (for parallel/cross-cluster tests)
guardian test bootstrap --clusters 3

# Without TUI
guardian test integration --no-tui
```

## Writing Tests

1. Use `cluster_ips[:5]` for a 5-node cluster (standard)
2. Use `create_vm_monitor(ips)` to track VM state
3. Use `cluster_logs` fixture for log-based assertions
4. Cleanup in `finally` blocks (reconnect partitioned VMs)
5. Use `restore_registry` if you modify registry config

```python
@pytest.mark.integration
def test_something(cluster_ips, create_vm_monitor, cluster_logs):
    test_ips = cluster_ips[:5]
    disconnected = []
    
    try:
        monitor = create_vm_monitor(test_ips)
        wait_for_ready(monitor)
        cluster_logs.set_test_context(test_ips)
        
        # Test logic...
        disconnect_vms([test_ips[0]])
        disconnected.append(test_ips[0])
        
        # Assertions...
        
    finally:
        reconnect_vms(disconnected)
```

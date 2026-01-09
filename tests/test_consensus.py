"""
Cluster consensus tests for TEE Guardian

Tests that verify distributed consensus works correctly across multiple nodes.
These tests require a multi-node TDX cluster to be running.
"""

import requests
import time
import pytest

# 5-node cluster for consensus testing
NODES = [
    "http://192.168.100.10:8443",
    "http://192.168.100.11:8443", 
    "http://192.168.100.12:8443",
    "http://192.168.100.13:8443",
    "http://192.168.100.14:8443",
]


def test_3_node_key_derivation_consensus():
    """3 nodes derive same key from same inputs"""
    key_request = {
        "namespace": "consensus_test", 
        "key_type": "volume", 
        "key_id": "3-node-consensus-key"
    }
    
    # Derive same key from first 3 nodes
    keys = []
    for node_url in NODES[:3]:
        response = requests.post(
            f"{node_url}/derive-key", 
            json=key_request, 
            timeout=15
        )
        assert response.status_code == 200
        
        data = response.json()
        keys.append(data["key"])
    
    # All 3 nodes should produce identical keys
    assert len(set(keys)) == 1
    assert len(keys[0]) > 0


def test_5_node_threshold_operations():
    """5-node cluster performs 3-of-5 threshold crypto operations"""
    # Test threshold signing operation
    message = "test_message_for_threshold_signing"
    
    # Request threshold signature from cluster
    response = requests.post(
        f"{NODES[0]}/threshold/sign", 
        json={
            "message": message,
            "threshold": 3,
            "total_nodes": 5
        }, 
        timeout=30
    )
    assert response.status_code == 200
    
    signature_data = response.json()
    assert "signature" in signature_data
    assert "participants" in signature_data
    assert len(signature_data["participants"]) >= 3  # At least 3 nodes participated
    
    # Verify the threshold signature
    verify_response = requests.post(
        f"{NODES[1]}/threshold/verify", 
        json={
            "message": message,
            "signature": signature_data["signature"],
            "participants": signature_data["participants"]
        }, 
        timeout=15
    )
    assert verify_response.status_code == 200
    
    verification = verify_response.json()
    assert verification["valid"] == True


def test_certificate_authority_consensus():
    """CA operations reach consensus across cluster"""
    # Generate CSR for testing
    csr = """-----BEGIN CERTIFICATE REQUEST-----
MIICWjCCAUICAQAwGTEXMBUGA1UEAwwOY29uc2Vuc3VzLXRlc3QwggEiMA0GCSqG
SIb3DQEBAQUAA4IBDwAwggEKAoIBAQDXYZ1234567890abcdefghijklmnop
-----END CERTIFICATE REQUEST-----"""
    
    # Issue certificate from first node
    issue_response = requests.post(
        f"{NODES[0]}/ca/issue", 
        data=csr, 
        headers={"Content-Type": "application/pkcs10"},
        timeout=20
    )
    assert issue_response.status_code == 200
    
    cert_data = issue_response.json()
    serial_number = cert_data["serial_number"]
    
    # Wait for consensus propagation
    time.sleep(5)
    
    # Verify certificate is known by other nodes
    for node_url in NODES[1:4]:  # Check 3 other nodes
        verify_response = requests.get(
            f"{node_url}/ca/certificate/{serial_number}", 
            timeout=10
        )
        assert verify_response.status_code == 200
        
        retrieved_cert = verify_response.json()
        assert retrieved_cert["serial_number"] == serial_number
        assert retrieved_cert["certificate"] == cert_data["certificate"]


def test_cluster_health_consensus():
    """All nodes in cluster report consistent health status"""
    health_statuses = []
    
    for node_url in NODES:
        try:
            response = requests.get(f"{node_url}/health", timeout=5)
            if response.status_code == 200:
                health_data = response.json()
                health_statuses.append({
                    "node": node_url,
                    "status": health_data["status"],
                    "cluster_size": health_data.get("cluster_size", 0),
                    "consensus_leader": health_data.get("consensus_leader", "unknown")
                })
        except requests.RequestException:
            # Node might be down, skip it
            continue
    
    # At least 3 nodes should be healthy (majority)
    healthy_nodes = [h for h in health_statuses if h["status"] == "healthy"]
    assert len(healthy_nodes) >= 3
    
    # All healthy nodes should report same cluster size
    cluster_sizes = [h["cluster_size"] for h in healthy_nodes]
    assert len(set(cluster_sizes)) == 1  # All report same cluster size
    
    # All healthy nodes should agree on consensus leader
    leaders = [h["consensus_leader"] for h in healthy_nodes]
    assert len(set(leaders)) == 1  # All agree on same leader


def test_distributed_key_storage_consensus():
    """Key storage operations are consistent across cluster"""
    key_data = {
        "namespace": "distributed_test",
        "key_id": "consensus-storage-test",
        "key_type": "application",
        "metadata": {"purpose": "consensus_testing"}
    }
    
    # Store key via first node
    store_response = requests.post(
        f"{NODES[0]}/keys/store", 
        json=key_data, 
        timeout=15
    )
    assert store_response.status_code == 200
    
    stored_key = store_response.json()
    key_id = stored_key["key_id"]
    
    # Wait for consensus propagation
    time.sleep(3)
    
    # Retrieve key from different nodes
    for node_url in NODES[1:4]:  # Check 3 other nodes
        retrieve_response = requests.get(
            f"{node_url}/keys/{key_id}", 
            timeout=10
        )
        assert retrieve_response.status_code == 200
        
        retrieved_key = retrieve_response.json()
        assert retrieved_key["key_id"] == key_id
        assert retrieved_key["namespace"] == key_data["namespace"]
        assert retrieved_key["metadata"] == key_data["metadata"]

"""
Basic functionality tests for TEE Guardian

Simple, direct tests that verify core functionality works on real TDX hardware.
Each test focuses on one specific scenario for maximum clarity.
"""

import requests
import pytest

# Single node for basic functionality tests
NODE_URL = "http://192.168.100.10:8443"


def test_node_boots_and_reports_healthy():
    """Node starts successfully and reports healthy status"""
    response = requests.get(f"{NODE_URL}/health", timeout=10)
    assert response.status_code == 200
    
    data = response.json()
    assert data["status"] == "healthy"
    assert "version" in data
    assert "uptime" in data


def test_real_tdx_quote_generation():
    """Generate real TDX quote and verify it contains expected data"""
    challenge = "test_challenge_real_hardware_123"
    
    response = requests.post(
        f"{NODE_URL}/attest", 
        json={"challenge": challenge}, 
        timeout=30
    )
    assert response.status_code == 200
    
    data = response.json()
    assert "quote" in data
    assert "rtmr_state" in data
    assert "report_data" in data
    assert "timestamp" in data
    
    # Verify quote is real TDX quote (not empty/mock)
    quote = data["quote"]
    assert len(quote) > 100  # Real TDX quotes are substantial
    assert isinstance(quote, str)  # Base64 encoded


def test_intel_dcap_verification():
    """Verify real TDX quote using Intel DCAP service"""
    # First generate a real quote
    challenge = "dcap_verification_test"
    
    attest_response = requests.post(
        f"{NODE_URL}/attest", 
        json={"challenge": challenge}, 
        timeout=30
    )
    assert attest_response.status_code == 200
    attestation = attest_response.json()
    
    # Now verify it using DCAP
    verify_response = requests.post(
        f"{NODE_URL}/verify", 
        json={
            "quote": attestation["quote"],
            "verifier": "dcap"
        }, 
        timeout=60
    )
    assert verify_response.status_code == 200
    
    verification = verify_response.json()
    assert verification["verified"] == True
    assert "tcb_status" in verification
    assert "quote_status" in verification


def test_intel_trust_authority_verification():
    """Verify real TDX quote using Intel Trust Authority"""
    # First generate a real quote
    challenge = "ita_verification_test"
    
    attest_response = requests.post(
        f"{NODE_URL}/attest", 
        json={"challenge": challenge}, 
        timeout=30
    )
    assert attest_response.status_code == 200
    attestation = attest_response.json()
    
    # Now verify it using Intel Trust Authority
    verify_response = requests.post(
        f"{NODE_URL}/verify", 
        json={
            "quote": attestation["quote"],
            "verifier": "intel_trust_authority"
        }, 
        timeout=60
    )
    assert verify_response.status_code == 200
    
    verification = verify_response.json()
    assert verification["verified"] == True
    assert "token" in verification  # ITA returns JWT token


def test_key_derivation_consistency():
    """Same inputs produce same keys across multiple requests"""
    key_request = {
        "namespace": "test", 
        "key_type": "volume", 
        "key_id": "consistency-test-key"
    }
    
    # Derive same key multiple times
    keys = []
    for i in range(3):
        response = requests.post(
            f"{NODE_URL}/derive-key", 
            json=key_request, 
            timeout=10
        )
        assert response.status_code == 200
        
        data = response.json()
        assert "key" in data
        keys.append(data["key"])
    
    # All keys should be identical
    assert len(set(keys)) == 1
    assert len(keys[0]) > 0  # Key should not be empty


def test_certificate_issuance():
    """Certificate authority issues valid certificates"""
    # Simple CSR for testing
    csr = """-----BEGIN CERTIFICATE REQUEST-----
MIICWjCCAUICAQAwFTETMBEGA1UEAwwKdGVzdC1jZXJ0MIIBIjANBgkqhkiG9w0B
AQEFAAOCAQ8AMIIBCgKCAQEA1234567890abcdefghijklmnopqrstuvwxyz
-----END CERTIFICATE REQUEST-----"""
    
    response = requests.post(
        f"{NODE_URL}/ca/issue", 
        data=csr, 
        headers={"Content-Type": "application/pkcs10"},
        timeout=15
    )
    assert response.status_code == 200
    
    data = response.json()
    assert "certificate" in data
    assert "serial_number" in data
    
    # Verify certificate format
    cert = data["certificate"]
    assert cert.startswith("-----BEGIN CERTIFICATE-----")
    assert cert.endswith("-----END CERTIFICATE-----")

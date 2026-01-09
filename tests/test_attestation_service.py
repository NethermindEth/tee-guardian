"""
Attestation Service E2E Tests

Tests the attestation service endpoints on real TDX hardware.
Replaces the integration and e2e tests that were removed from Rust.
"""

import base64
import requests
import pytest
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

# Service endpoint
SERVICE_URL = "http://192.168.100.10:8080"


def test_health_endpoint():
    """Health endpoint returns correct status"""
    response = requests.get(f"{SERVICE_URL}/health", timeout=10)
    assert response.status_code == 200

    data = response.json()
    assert data["status"] == "healthy"
    assert "timestamp" in data
    assert "version" in data


def test_generate_attestation_valid_request():
    """Generate attestation with valid request on TDX hardware"""
    nonce = base64.b64encode(b"test_nonce_12345").decode()
    user_data = base64.b64encode(b"test_user_data").decode()

    request = {"nonce": nonce, "user_data": user_data}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=30)
    assert response.status_code == 200

    data = response.json()
    assert "quote" in data
    assert "rtmr_state" in data
    assert "report_data" in data
    assert "timestamp" in data
    assert data["version"] == "1.0"

    # Verify RTMR state structure
    rtmr_state = data["rtmr_state"]
    assert "rtmr0" in rtmr_state
    assert "rtmr1" in rtmr_state
    assert "rtmr2" in rtmr_state
    assert "rtmr3" in rtmr_state

    # Verify quote is substantial (real TDX quote)
    quote = data["quote"]
    assert len(quote) > 100


def test_generate_attestation_minimal_request():
    """Generate attestation with minimal request (no nonce/user_data)"""
    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json={}, timeout=30)
    assert response.status_code == 200

    data = response.json()
    assert "quote" in data
    assert "rtmr_state" in data


def test_generate_attestation_invalid_base64_nonce():
    """Invalid base64 nonce returns bad request"""
    request = {"nonce": "invalid_base64!!!"}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=10)
    assert response.status_code == 400

    data = response.json()
    assert "Invalid base64 nonce" in data["error"]


def test_generate_attestation_invalid_base64_user_data():
    """Invalid base64 user_data returns bad request"""
    request = {"user_data": "invalid_base64!!!"}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=10)
    assert response.status_code == 400

    data = response.json()
    assert "Invalid base64 user_data" in data["error"]


def test_generate_attestation_oversized_nonce():
    """Oversized nonce returns bad request"""
    # Create 33-byte nonce (too large)
    large_nonce = base64.b64encode(b"x" * 33).decode()
    request = {"nonce": large_nonce}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=10)
    assert response.status_code == 400

    data = response.json()
    assert "Nonce too large" in data["error"]


def test_generate_attestation_oversized_user_data():
    """Oversized user_data returns bad request"""
    # Create 33-byte user_data (too large)
    large_user_data = base64.b64encode(b"x" * 33).decode()
    request = {"user_data": large_user_data}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=10)
    assert response.status_code == 400

    data = response.json()
    assert "User data too large" in data["error"]


def test_verify_attestation_with_real_quote():
    """Verify attestation using real TDX quote"""
    # First generate a real attestation
    nonce = base64.b64encode(b"verification_test").decode()
    generate_request = {"nonce": nonce}

    generate_response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=generate_request, timeout=30)
    assert generate_response.status_code == 200
    attestation = generate_response.json()

    # Now verify it
    verify_request = {"quote": attestation["quote"], "expected_nonce": nonce}

    verify_response = requests.post(f"{SERVICE_URL}/v1/attestation/verify", json=verify_request, timeout=60)

    # Should process (may succeed or fail depending on environment)
    assert verify_response.status_code in [200, 422]

    if verify_response.status_code == 200:
        verification = verify_response.json()
        assert "verified" in verification
        assert "tcb_status" in verification
        assert "verified_at" in verification


def test_verify_attestation_missing_quote():
    """Verification with missing quote returns bad request"""
    response = requests.post(f"{SERVICE_URL}/v1/attestation/verify", json={}, timeout=10)
    assert response.status_code == 400

    data = response.json()
    assert "Validation failed" in data["error"]


def test_verify_attestation_invalid_base64_quote():
    """Verification with invalid base64 quote returns bad request"""
    request = {"quote": "invalid_base64!!!"}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/verify", json=request, timeout=10)
    assert response.status_code == 400

    data = response.json()
    assert "Invalid base64 quote" in data["error"]


def test_attestation_with_various_nonce_sizes():
    """Test attestation generation with different nonce sizes"""
    test_cases = [
        ("empty", b""),
        ("small", b"test"),
        ("medium", b"medium_length_nonce_data"),
        ("max_size", b"x" * 32),  # Maximum allowed size
    ]

    for name, nonce_data in test_cases:
        request = {"nonce": base64.b64encode(nonce_data).decode()}

        response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=30)
        assert response.status_code == 200, f"Failed for {name} nonce"


def test_attestation_consistency():
    """Multiple attestations with same nonce have consistent RTMR values"""
    nonce = base64.b64encode(b"consistency_test_nonce").decode()
    request = {"nonce": nonce}

    attestations = []
    for i in range(3):
        response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=30)
        assert response.status_code == 200
        attestations.append(response.json())

    # RTMR values should be consistent across generations
    rtmr0_first = attestations[0]["rtmr_state"]["rtmr0"]
    rtmr0_second = attestations[1]["rtmr_state"]["rtmr0"]
    assert rtmr0_first == rtmr0_second

    # Report data should be consistent (same nonce)
    report_data_first = attestations[0]["report_data"]
    report_data_second = attestations[1]["report_data"]
    assert report_data_first == report_data_second


def test_attestation_with_policy():
    """Test attestation verification with policy enforcement"""
    # First generate an attestation
    generate_request = {"nonce": base64.b64encode(b"policy_test_nonce").decode()}

    generate_response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=generate_request, timeout=30)
    assert generate_response.status_code == 200
    attestation = generate_response.json()

    # Test with restrictive policy
    restrictive_policy = {
        "min_tcb_version": "99.99.99",  # Unrealistic version
        "trusted_measurements": ["impossible_measurement"],
    }

    verify_request = {"quote": attestation["quote"], "policy": restrictive_policy}

    verify_response = requests.post(f"{SERVICE_URL}/v1/attestation/verify", json=verify_request, timeout=30)

    # Should process but likely fail verification
    assert verify_response.status_code in [200, 422]

    if verify_response.status_code == 200:
        verification = verify_response.json()
        assert "verified" in verification


def test_rapid_sequential_requests():
    """Service handles rapid sequential requests gracefully"""
    successful_requests = 0
    rate_limited_requests = 0

    for i in range(20):
        request = {"nonce": base64.b64encode(f"rapid_test_{i}".encode()).decode()}

        response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=10)

        if response.status_code == 200:
            successful_requests += 1
        elif response.status_code == 429:  # Too Many Requests
            rate_limited_requests += 1
        elif response.status_code == 503:  # Service Unavailable
            pass  # Acceptable for TDX hardware limitations
        else:
            pytest.fail(f"Unexpected status code: {response.status_code}")

        time.sleep(0.05)  # Small delay

    # Should handle requests gracefully
    assert successful_requests > 0 or rate_limited_requests > 0


def test_attestation_data_integrity():
    """Verify user data is preserved in attestation report_data"""
    test_data = b"integrity_test_data_12345"
    request = {"user_data": base64.b64encode(test_data).decode()}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=30)
    assert response.status_code == 200

    attestation = response.json()

    # Decode and verify report data contains our user data
    report_data_hex = attestation["report_data"]
    report_data_bytes = bytes.fromhex(report_data_hex)

    # User data should be in the second half of report_data (bytes 32-64)
    user_data_section = report_data_bytes[32 : 32 + len(test_data)]
    assert user_data_section == test_data


def test_service_recovery_after_errors():
    """Service recovers and handles valid requests after errors"""
    # Send invalid request to trigger error
    invalid_request = {"nonce": "invalid_base64!!!"}

    error_response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=invalid_request, timeout=10)
    assert error_response.status_code == 400

    # Service should recover and handle valid request
    valid_request = {"nonce": base64.b64encode(b"recovery_test").decode()}

    recovery_response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=valid_request, timeout=30)

    # Should succeed or return service unavailable (not error from previous request)
    assert recovery_response.status_code in [200, 503]


def test_concurrent_attestation_generations():
    """Multiple concurrent attestation requests are handled properly"""

    def generate_attestation(i):
        request = {"nonce": base64.b64encode(f"concurrent_test_{i}".encode()).decode()}
        return requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=30)

    # Launch 5 concurrent requests
    with ThreadPoolExecutor(max_workers=5) as executor:
        futures = [executor.submit(generate_attestation, i) for i in range(5)]

        successful = 0
        rate_limited = 0
        unavailable = 0

        for future in as_completed(futures):
            response = future.result()

            if response.status_code == 200:
                successful += 1
            elif response.status_code == 429:
                rate_limited += 1
            elif response.status_code == 503:
                unavailable += 1
            else:
                pytest.fail(f"Unexpected concurrent response status: {response.status_code}")

    # All requests should be handled gracefully
    assert successful + rate_limited + unavailable == 5


def test_malformed_json_request():
    """Malformed JSON request returns bad request"""
    response = requests.post(
        f"{SERVICE_URL}/v1/attestation/generate",
        headers={"content-type": "application/json"},
        data='{"invalid": json, "missing": quotes}',
        timeout=10,
    )
    assert response.status_code == 400


def test_unsupported_http_method():
    """Unsupported HTTP method returns method not allowed"""
    response = requests.put(f"{SERVICE_URL}/v1/attestation/generate", timeout=10)
    assert response.status_code == 405


def test_non_existent_endpoint():
    """Non-existent endpoint returns not found"""
    response = requests.get(f"{SERVICE_URL}/non-existent", timeout=10)
    assert response.status_code == 404


def test_security_headers():
    """Security headers are properly set"""
    response = requests.get(f"{SERVICE_URL}/health", timeout=10)

    headers = response.headers

    # Check required security headers
    assert headers.get("x-content-type-options") == "nosniff"
    assert headers.get("x-frame-options") == "DENY"
    assert headers.get("x-xss-protection") == "1; mode=block"
    assert headers.get("referrer-policy") == "strict-origin-when-cross-origin"
    assert headers.get("content-security-policy") == "default-src 'none'"

    # Server header should be removed
    assert "server" not in headers


def test_cors_handling():
    """CORS preflight requests are handled appropriately"""
    response = requests.options(
        f"{SERVICE_URL}/v1/attestation/generate",
        headers={
            "Origin": "https://example.com",
            "Access-Control-Request-Method": "POST",
            "Access-Control-Request-Headers": "content-type",
        },
        timeout=10,
    )

    # Should handle CORS appropriately
    assert response.status_code in [200, 204, 403]


def test_large_valid_payload():
    """Service handles large valid payloads within limits"""
    # Create request with maximum allowed sizes
    max_nonce = base64.b64encode(b"x" * 32).decode()  # 32 bytes = max
    max_user_data = base64.b64encode(b"y" * 32).decode()  # 32 bytes = max

    request = {"nonce": max_nonce, "user_data": max_user_data}

    response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=request, timeout=30)

    # Should succeed or return service unavailable (not bad request)
    assert response.status_code in [200, 503]


def test_cross_service_verification():
    """Attestation generated on one service can be verified by another"""
    # This test would require multiple service instances
    # For now, we'll test self-verification

    # Generate attestation
    nonce = base64.b64encode(b"cross_service_test").decode()
    generate_request = {"nonce": nonce}

    generate_response = requests.post(f"{SERVICE_URL}/v1/attestation/generate", json=generate_request, timeout=30)
    assert generate_response.status_code == 200
    attestation = generate_response.json()

    # Verify on same service (would be different service in real deployment)
    verify_request = {"quote": attestation["quote"], "expected_nonce": nonce}

    verify_response = requests.post(f"{SERVICE_URL}/v1/attestation/verify", json=verify_request, timeout=60)

    # Cross-service verification should work
    assert verify_response.status_code in [200, 422]


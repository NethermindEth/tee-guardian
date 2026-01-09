// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! API response types for attestation protocol.

use serde::{Deserialize, Serialize};

/// Response from Phase 1 registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationResponse {
    /// Challenge nonce to include in Phase 2 quote (hex-encoded).
    pub challenge: String,
    /// Challenge expiration (Unix timestamp).
    pub expires_at: u64,
}

/// Response from Phase 2 verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResponse {
    /// Whether verification succeeded.
    pub verified: bool,
    /// Measurement hash (hex-encoded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement_hash: Option<String>,
    /// TCB status from attestation verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcb_status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registration_response_serialization() {
        let response = RegistrationResponse { challenge: "deadbeef".to_string(), expires_at: 1234567890 };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: RegistrationResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.challenge, "deadbeef");
        assert_eq!(parsed.expires_at, 1234567890);
    }

    #[test]
    fn test_verification_response_serialization() {
        let response = VerificationResponse {
            verified: true,
            measurement_hash: Some("abc123".to_string()),
            tcb_status: Some("UpToDate".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("verified"));
        assert!(json.contains("UpToDate"));
    }

    #[test]
    fn test_verification_response_skips_none() {
        let response = VerificationResponse { verified: false, measurement_hash: None, tcb_status: None };

        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("measurement_hash"));
        assert!(!json.contains("tcb_status"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Gossip protocol types.
//!
//! Types for peer advertisements and API responses.

use peer_registry::types::NodeId;
use peer_registry::{detect_tee_type, Bytes32, SnpReport, TdxQuote, Tee};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;

use tracing::debug;

/// Peer advertisement for gossip protocol.
///
/// Contains discovery metadata for a node, authenticated by TEE evidence.
/// The evidence binds the advertised addresses to the TEE instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAdvertisement {
    /// Node ID (derived from IP address)
    pub node_id: NodeId,

    /// RAFT address (IP:port for RAFT RPC)
    pub raft_address: SocketAddr,

    /// API address (IP:port for Guardian API)
    pub api_address: SocketAddr,

    /// Measurement hash (SHA256 of RTMRs) - for quick pre-filtering
    pub measurement_hash: String,

    /// Instance ID (hex-encoded 32 bytes) - from REPORTDATA[32:64] (TDX) or REPORT_ID (SNP)
    pub instance_id: String,

    /// Unix timestamp of advertisement creation
    pub timestamp: u64,

    /// Base64-encoded TEE evidence (TDX quote or SEV-SNP report).
    ///
    /// REPORTDATA layout:
    /// - `[0:32]`: SHA256(raft_address || api_address) - address binding
    /// - `[32:64]`: instance_id (TDX) or zeros (SEV-SNP uses REPORT_ID)
    #[serde(default)]
    pub evidence: String,
}

impl PeerAdvertisement {
    /// Create a peer advertisement with TEE evidence.
    #[must_use]
    pub fn new(
        node_id: NodeId,
        raft_address: SocketAddr,
        api_address: SocketAddr,
        measurement_hash: String,
        instance_id: String,
        evidence: String,
    ) -> Self {
        Self {
            node_id,
            raft_address,
            api_address,
            measurement_hash,
            instance_id,
            timestamp: common::unix_timestamp_secs(),
            evidence,
        }
    }

    /// Compute the address binding hash for REPORTDATA[0:32].
    ///
    /// This binds the advertisement to specific network addresses,
    /// preventing replay attacks with different addresses.
    #[must_use]
    pub fn compute_address_binding(raft: &SocketAddr, api: &SocketAddr) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(raft.to_string().as_bytes());
        hasher.update(api.to_string().as_bytes());
        hasher.finalize().into()
    }

    /// Validate advertisement format (basic checks, no crypto).
    ///
    /// Checks:
    /// - instance_id is valid hex (64 chars = 32 bytes)
    /// - measurement_hash is valid hex (64 chars = 32 bytes)
    /// - timestamp is not in the future
    pub fn validate_format(&self) -> Result<(), String> {
        // Validate instance_id format
        if self.instance_id.len() != 64 {
            return Err(format!("Invalid instance_id length: {} (expected 64 hex chars)", self.instance_id.len()));
        }
        hex::decode(&self.instance_id).map_err(|e| format!("Invalid instance_id hex: {}", e))?;

        // Validate measurement_hash format
        if self.measurement_hash.len() != 64 {
            return Err(format!(
                "Invalid measurement_hash length: {} (expected 64 hex chars)",
                self.measurement_hash.len()
            ));
        }
        hex::decode(&self.measurement_hash).map_err(|e| format!("Invalid measurement_hash hex: {}", e))?;

        // Check timestamp not in future (with 5 minute tolerance)
        if self.timestamp > common::unix_timestamp_secs() + 300 {
            return Err("Advertisement timestamp is in the future".to_string());
        }

        Ok(())
    }

    /// Verify TEE evidence and address binding.
    ///
    /// This verifies:
    /// 1. The quote/report has a valid DCAP/VCEK signature (proves TEE)
    /// 2. REPORTDATA[0:32] matches SHA256(raft_address || api_address)
    /// 3. REPORTDATA[32:64] matches the claimed instance_id
    ///
    /// Does NOT verify measurements - that's done during full attestation.
    pub async fn verify_evidence(&self) -> Result<(), String> {
        // TEE evidence is required
        if self.evidence.is_empty() {
            return Err("TEE evidence required".to_string());
        }

        // Decode evidence to detect TEE type
        let evidence_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &self.evidence)
            .map_err(|e| format!("Invalid evidence base64: {e}"))?;

        let evidence_json: serde_json::Value =
            serde_json::from_slice(&evidence_bytes).map_err(|e| format!("Invalid evidence JSON: {e}"))?;

        let tee = detect_tee_type(&evidence_json).ok_or_else(|| "Cannot detect TEE type from evidence".to_string())?;

        // Compute expected address binding
        let expected_binding = Bytes32::from(Self::compute_address_binding(&self.raft_address, &self.api_address));

        // Decode expected instance_id
        let expected_instance_id: [u8; 32] = hex::decode(&self.instance_id)
            .map_err(|e| format!("Invalid instance_id hex: {e}"))?
            .try_into()
            .map_err(|_| "Instance ID must be 32 bytes")?;

        // Verify based on TEE type
        match tee {
            Tee::Tdx => {
                let quote =
                    TdxQuote::from_json(evidence_json).map_err(|e| format!("Failed to parse TDX quote: {e}"))?;

                // Verify DCAP signature chain
                let verified =
                    quote.verify_signature().await.map_err(|e| format!("TDX signature verification failed: {e}"))?;

                // Verify address binding (REPORTDATA[0:32])
                verified.verify_report_data(&expected_binding).map_err(|e| format!("Address binding mismatch: {e}"))?;

                // Verify instance_id (REPORTDATA[32:64])
                let actual_instance_id = verified.instance_id();
                if actual_instance_id.as_bytes() != &expected_instance_id {
                    return Err(format!(
                        "Instance ID mismatch: expected {}, got {}",
                        self.instance_id,
                        actual_instance_id.to_hex()
                    ));
                }

                debug!(
                    node_id = self.node_id,
                    instance_id = %actual_instance_id.short(),
                    "TDX advertisement verified"
                );
            }
            Tee::Snp => {
                let report =
                    SnpReport::from_json(evidence_json).map_err(|e| format!("Failed to parse SEV-SNP report: {e}"))?;

                // Verify VCEK/VLEK signature chain
                let verified = report
                    .verify_signature()
                    .await
                    .map_err(|e| format!("SEV-SNP signature verification failed: {e}"))?;

                // Verify address binding (REPORT_DATA[0:32])
                verified.verify_report_data(&expected_binding).map_err(|e| format!("Address binding mismatch: {e}"))?;

                // For SEV-SNP, instance_id comes from REPORT_ID, not REPORT_DATA[32:64]
                // Just verify the claimed instance_id matches what's in the report
                let actual_instance_id = verified.instance_id();
                if actual_instance_id.as_bytes() != &expected_instance_id {
                    return Err(format!(
                        "Instance ID mismatch: expected {}, got {}",
                        self.instance_id,
                        actual_instance_id.to_hex()
                    ));
                }

                debug!(
                    node_id = self.node_id,
                    instance_id = %actual_instance_id.short(),
                    "SEV-SNP advertisement verified"
                );
            }
            _ => {
                return Err(format!("Unsupported TEE type: {tee:?}"));
            }
        }

        Ok(())
    }
}

/// Response to peer list request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerListResponse {
    /// List of peer advertisements
    pub peers: Vec<PeerAdvertisement>,
}

/// Response to advertisement submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvertiseResponse {
    /// Whether the advertisement was accepted
    pub accepted: bool,
    /// Reason if not accepted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_address_binding() {
        let raft: SocketAddr = "192.168.1.20:8444".parse().unwrap();
        let api: SocketAddr = "192.168.1.20:8443".parse().unwrap();

        let binding1 = PeerAdvertisement::compute_address_binding(&raft, &api);
        let binding2 = PeerAdvertisement::compute_address_binding(&raft, &api);

        // Deterministic
        assert_eq!(binding1, binding2);

        // Different addresses produce different binding
        let raft2: SocketAddr = "192.168.1.21:8444".parse().unwrap();
        let binding3 = PeerAdvertisement::compute_address_binding(&raft2, &api);
        assert_ne!(binding1, binding3);
    }

    #[test]
    fn test_peer_advertisement_validate_format() {
        let ad = PeerAdvertisement::new(
            123456,
            "192.168.1.20:8444".parse().unwrap(),
            "192.168.1.20:8443".parse().unwrap(),
            "a".repeat(64), // valid 32-byte hex
            "b".repeat(64), // valid 32-byte hex
            String::new(),  // no evidence
        );

        assert!(ad.validate_format().is_ok());
    }

    #[test]
    fn test_peer_advertisement_invalid_instance_id() {
        let ad = PeerAdvertisement::new(
            123456,
            "192.168.1.20:8444".parse().unwrap(),
            "192.168.1.20:8443".parse().unwrap(),
            "a".repeat(64),
            "short".to_string(), // too short
            String::new(),
        );

        assert!(ad.validate_format().is_err());
    }

    #[test]
    fn test_peer_advertisement_invalid_measurement_hash() {
        let ad = PeerAdvertisement::new(
            123456,
            "192.168.1.20:8444".parse().unwrap(),
            "192.168.1.20:8443".parse().unwrap(),
            "zzzz".to_string(), // invalid hex
            "b".repeat(64),
            String::new(),
        );

        assert!(ad.validate_format().is_err());
    }
}

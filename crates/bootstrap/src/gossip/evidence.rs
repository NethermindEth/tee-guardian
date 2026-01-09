// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE evidence generation for gossip advertisements.

use super::types::PeerAdvertisement;
use base64::Engine;
use peer_registry::{detect_available_tee, generate_evidence};
use std::net::SocketAddr;
use tracing::info;

/// Generate TEE evidence for gossip advertisements.
///
/// Creates a TDX quote or SEV-SNP report with REPORTDATA:
/// - `[0:32]`: SHA256(raft_address || api_address) - address binding
/// - `[32:64]`: instance_id
///
/// Returns error if no TEE platform is available.
pub async fn generate_gossip_evidence(
    raft_address: &SocketAddr,
    api_address: &SocketAddr,
    instance_id: &str,
) -> Result<String, String> {
    // Check if TEE is available
    let tee =
        detect_available_tee().ok_or_else(|| "No TEE platform available (TDX or SEV-SNP required)".to_string())?;

    // Compute address binding
    let binding = PeerAdvertisement::compute_address_binding(raft_address, api_address);

    // Decode instance_id
    let instance_bytes = hex::decode(instance_id).map_err(|e| format!("Invalid instance_id hex: {e}"))?;

    if instance_bytes.len() != 32 {
        return Err("Instance ID must be 32 bytes".to_string());
    }

    // Build REPORTDATA
    let mut report_data = [0u8; 64];
    report_data[0..32].copy_from_slice(&binding);
    report_data[32..64].copy_from_slice(&instance_bytes);

    info!(
        tee = ?tee,
        binding_prefix = hex::encode(&binding[0..8]),
        "Generating gossip evidence"
    );

    // Generate evidence using peer_registry
    let evidence = generate_evidence(tee, &report_data).await.map_err(|e| e.to_string())?;

    // Serialize to base64
    let json = serde_json::to_vec(&evidence).map_err(|e| format!("Failed to serialize evidence: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&json))
}

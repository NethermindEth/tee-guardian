// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Guardian Node API - Attestation, peer discovery, and gossip endpoints.
//!
//! # Endpoints
//!
//! ## Attestation (Two-Phase Protocol)
//! - `POST /attestation/{namespace}/register` - Phase 1: Register and get challenge
//! - `POST /attestation/{namespace}/verify` - Phase 2: Verify with challenge
//!
//! ## Legacy Attestation (for generating quotes on request)
//! - `GET /attestation/quote/{nonce}` - Generate TDX quote with nonce
//!
//! ## Gossip
//! - `GET /guardian_node/peers` - List known peer advertisements
//! - `POST /guardian_node/advertise` - Receive peer advertisement

use crate::attestation::{handle_register, handle_verify, AttestationApiState, RegisterRequest, VerifyRequest};
use crate::gossip::{AdvertiseResponse, GossipConfig, GossipedPeerCache, PeerAdvertisement, PeerListResponse};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
pub use common::ApiError;
use peer_registry::types::NodeId;
use peer_registry::{is_tee_available, Tee};
use peer_registry::{RegistrationResponse, VerificationResponse};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

/// Shared state for Guardian Node API.
#[derive(Clone)]
pub struct GuardianNodeApiState {
    /// This node's ID.
    pub node_id: NodeId,
    /// Attestation state (PeerRegistry + MeasurementClient).
    pub attestation: AttestationApiState,
    /// Gossip cache for peer advertisements.
    pub gossip_cache: Arc<GossipedPeerCache>,
    /// Gossip configuration.
    pub gossip_config: GossipConfig,
}

impl GuardianNodeApiState {
    /// Create new API state.
    pub fn new(
        node_id: NodeId,
        attestation: AttestationApiState,
        gossip_cache: Arc<GossipedPeerCache>,
        gossip_config: GossipConfig,
    ) -> Self {
        Self { node_id, attestation, gossip_cache, gossip_config }
    }
}

/// Response for generating a quote.
#[derive(Debug, Serialize, Deserialize)]
pub struct QuoteResponse {
    /// Base64-encoded TDX evidence.
    pub evidence: String,
    /// Unix timestamp when quote was generated.
    pub timestamp: u64,
}

/// Start the Guardian Node API server.
pub async fn start_guardian_node_api(state: GuardianNodeApiState, bind_addr: &str, port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", bind_addr, port).parse()?;

    let app = create_guardian_node_router(state);

    info!("Starting Guardian Node API server on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

/// Create the Guardian Node API router.
pub fn create_guardian_node_router(state: GuardianNodeApiState) -> Router {
    Router::new()
        // Two-phase attestation protocol
        .route("/attestation/:namespace/register", post(register_handler))
        .route("/attestation/:namespace/verify", post(verify_handler))
        // Legacy quote generation (for candidates to generate quotes)
        .route("/attestation/quote/:nonce", get(quote_handler))
        // Gossip endpoints
        .route("/guardian_node/peers", get(peers_handler))
        .route("/guardian_node/advertise", post(advertise_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Convert verification error to API error with appropriate status.
fn verification_error_to_api(e: peer_registry::VerificationError) -> ApiError {
    let msg = e.to_string();
    if msg.contains("not trusted") || msg.contains("not found") {
        ApiError::Forbidden(msg)
    } else if msg.contains("expired") || msg.contains("mismatch") {
        ApiError::BadRequest(msg)
    } else {
        ApiError::Internal(msg)
    }
}

/// Phase 1: Register TEE and issue challenge.
///
/// POST /attestation/{namespace}/register
async fn register_handler(
    State(state): State<GuardianNodeApiState>,
    Path(namespace): Path<String>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegistrationResponse>, ApiError> {
    let response = handle_register(&state.attestation, &namespace, request).await.map_err(verification_error_to_api)?;

    Ok(Json(response))
}

/// Phase 2: Verify challenge-bound attestation.
///
/// POST /attestation/{namespace}/verify
async fn verify_handler(
    State(state): State<GuardianNodeApiState>,
    Path(namespace): Path<String>,
    Json(request): Json<VerifyRequest>,
) -> Result<Json<VerificationResponse>, ApiError> {
    let response = handle_verify(&state.attestation, &namespace, request).await.map_err(verification_error_to_api)?;

    Ok(Json(response))
}

/// Generate TDX quote with nonce.
///
/// GET /attestation/quote/{nonce}
///
/// Used by candidates to generate quotes for the two-phase protocol.
async fn quote_handler(Path(nonce): Path<String>) -> Result<Json<QuoteResponse>, ApiError> {
    use attester::{tdx::TdxAttester, Attester};
    use base64::Engine;

    // Check TDX availability
    if !is_tee_available(Tee::Tdx) {
        return Err(ApiError::ServiceUnavailable("TDX platform not available".to_string()));
    }

    // Decode nonce from hex
    let nonce_bytes = hex::decode(&nonce).map_err(|e| ApiError::BadRequest(format!("Invalid hex nonce: {}", e)))?;

    if nonce_bytes.len() > 64 {
        return Err(ApiError::BadRequest("Nonce must be 64 bytes or less".to_string()));
    }

    // Pad to 64 bytes for report_data
    let mut report_data = vec![0u8; 64];
    let copy_len = nonce_bytes.len().min(64);
    report_data[..copy_len].copy_from_slice(&nonce_bytes[..copy_len]);

    // Generate attestation using attester crate directly
    let attester = TdxAttester::default();
    let raw_evidence = attester
        .get_evidence(report_data)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to generate attestation: {}", e)))?;

    // Encode as base64 for transmission
    let evidence = base64::prelude::BASE64_STANDARD.encode(raw_evidence.to_string().as_bytes());

    let timestamp = common::unix_timestamp_secs();

    Ok(Json(QuoteResponse { evidence, timestamp }))
}

/// List known peer advertisements.
///
/// GET /guardian_node/peers
async fn peers_handler(State(state): State<GuardianNodeApiState>) -> Result<Json<PeerListResponse>, ApiError> {
    let peers = state.gossip_cache.get_all_advertisements().await;

    let peers: Vec<PeerAdvertisement> = peers.into_iter().take(state.gossip_config.max_peers_per_message).collect();

    Ok(Json(PeerListResponse { peers }))
}

/// Receive and verify peer advertisement.
///
/// POST /guardian_node/advertise
///
/// Verifies TEE evidence (DCAP/VCEK signature chain) and address binding
/// before accepting the advertisement into the gossip cache.
async fn advertise_handler(
    State(state): State<GuardianNodeApiState>,
    Json(ad): Json<PeerAdvertisement>,
) -> Result<Json<AdvertiseResponse>, ApiError> {
    // Validate format
    if let Err(e) = ad.validate_format() {
        return Ok(Json(AdvertiseResponse { accepted: false, reason: Some(format!("Invalid format: {}", e)) }));
    }

    // Check timestamp freshness
    let now = common::unix_timestamp_secs();
    let age_hours = (now.saturating_sub(ad.timestamp)) / 3600;

    if age_hours > state.gossip_config.max_advertisement_age_hours {
        return Ok(Json(AdvertiseResponse {
            accepted: false,
            reason: Some(format!("Advertisement too old: {} hours", age_hours)),
        }));
    }

    // Verify TEE evidence first - establishes cryptographic identity
    if let Err(e) = ad.verify_evidence().await {
        return Ok(Json(AdvertiseResponse {
            accepted: false,
            reason: Some(format!("Evidence verification failed: {}", e)),
        }));
    }

    // After verification, check if someone is impersonating us.
    // This check comes after verify_evidence() so we have their verified instance_id
    // for forensics, and they can't spam warnings without valid TEE quotes.
    if ad.node_id == state.node_id {
        warn!(
            claimed_node_id = ad.node_id,
            instance_id = %ad.instance_id,
            "Rejected advertisement claiming our node_id"
        );
        return Ok(Json(AdvertiseResponse {
            accepted: false,
            reason: Some("Cannot advertise as this node".to_string()),
        }));
    }

    state.gossip_cache.add_verified(ad.clone()).await;
    info!(node_id = ad.node_id, "Accepted verified peer advertisement");

    Ok(Json(AdvertiseResponse { accepted: true, reason: None }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_response_serialization() {
        let response = QuoteResponse { evidence: "base64data".to_string(), timestamp: 1234567890 };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("evidence"));
        assert!(json.contains("1234567890"));
    }
}

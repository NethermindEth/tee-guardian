// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE attestation API and event handling.
//!
//! This module provides the attestation endpoints for the two-phase authentication
//! protocol, plus background tasks for event handling and cleanup.
//!
//! # Protocol
//!
//! ```text
//! Candidate                                    Cluster
//!    │                                            │
//!    │  POST /attestation/{ns}/register           │
//!    │  { evidence: <quote with instance_id> }    │
//!    │ ──────────────────────────────────────────>│
//!    │                                            │
//!    │<────────────────────────────────────────── │
//!    │  { instance_id, challenge, expires_at }    │
//!    │                                            │
//!    │  POST /attestation/{ns}/verify             │
//!    │  { instance_id, evidence: <challenge>}     │
//!    │ ──────────────────────────────────────────>│
//!    │                                            │
//!    │<────────────────────────────────────────── │
//!    │  { verified: true, ... }                   │
//! ```
//!
//! # Event Handling
//!
//! The `AttestationEventHandler` subscribes to `PeerRegistry` events and updates
//! `TrustedPeerRegistry` when TEEs are verified in the guardian namespace.

use crate::trusted_peers::TrustedPeerRegistry;
use peer_registry::{
    ip_to_node_id, register_tee, verify_tee, MeasurementRegistryClient, PeerRegistry, PeerRegistryEvent,
    RegistrationResponse, VerificationResponse,
};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Interval for cleaning up expired pending challenges.
const CLEANUP_INTERVAL_SECS: u64 = 30;

/// Shared state for attestation API.
#[derive(Clone)]
pub struct AttestationApiState {
    /// PeerRegistry for tracking TEE state.
    pub peer_registry: PeerRegistry,
    /// Client for querying measurement whitelist.
    pub measurement_client: Arc<MeasurementRegistryClient>,
}

impl AttestationApiState {
    /// Create new attestation API state.
    pub fn new(registry_url: String) -> Self {
        Self {
            peer_registry: PeerRegistry::new(),
            measurement_client: Arc::new(MeasurementRegistryClient::new(registry_url)),
        }
    }

    /// Get a reference to the peer registry.
    pub fn registry(&self) -> &PeerRegistry {
        &self.peer_registry
    }

    /// Subscribe to peer registry events.
    pub fn subscribe(&self) -> broadcast::Receiver<PeerRegistryEvent> {
        self.peer_registry.subscribe()
    }
}

/// Request body for Phase 1 registration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegisterRequest {
    /// Base64-encoded TDX evidence (initial quote).
    pub evidence: String,
}

/// Request body for Phase 2 verification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerifyRequest {
    /// Base64-encoded TDX evidence (challenge-bound quote).
    /// Instance ID is extracted from the quote's REPORTDATA[32:64].
    pub evidence: String,
}

/// Error type for attestation API (re-export for convenience).
pub type AttestationError = peer_registry::VerificationError;

/// Phase 1: Register TEE and issue challenge.
///
/// Called by candidate TEE to initiate authentication.
pub async fn handle_register(
    state: &AttestationApiState,
    namespace: &str,
    request: RegisterRequest,
) -> Result<RegistrationResponse, AttestationError> {
    register_tee(&state.peer_registry, &state.measurement_client, namespace, &request.evidence).await
}

/// Phase 2: Verify challenge-bound attestation.
///
/// Called by candidate TEE to complete authentication.
pub async fn handle_verify(
    state: &AttestationApiState,
    namespace: &str,
    request: VerifyRequest,
) -> Result<VerificationResponse, AttestationError> {
    verify_tee(&state.peer_registry, namespace, &request.evidence).await
}

/// Background task that handles PeerRegistry events for a specific namespace.
///
/// When a TEE is verified in the target namespace, it's added to `TrustedPeerRegistry`.
/// When a TEE is revoked, it's removed.
pub struct AttestationEventHandler {
    namespace: String,
    events: broadcast::Receiver<PeerRegistryEvent>,
}

impl AttestationEventHandler {
    /// Create a new event handler for the given namespace.
    pub fn new(namespace: String, events: broadcast::Receiver<PeerRegistryEvent>) -> Self {
        Self { namespace, events }
    }

    /// Run the event handler forever.
    ///
    /// Listens for PeerRegistry events and updates TrustedPeerRegistry accordingly.
    pub async fn run(mut self) -> ! {
        info!(namespace = %self.namespace, "Attestation event handler started");

        loop {
            match self.events.recv().await {
                Ok(event) => {
                    // Filter to our namespace
                    if event.namespace() != self.namespace {
                        continue;
                    }

                    self.handle_event(event).await;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(
                        namespace = %self.namespace,
                        missed = n,
                        "Event handler lagged, missed events"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => {
                    panic!(
                        "BUG: PeerRegistry event channel closed for namespace '{}' - \
                         this controls cluster membership and must remain active",
                        self.namespace
                    );
                }
            }
        }
    }

    async fn handle_event(&self, event: PeerRegistryEvent) {
        match event {
            PeerRegistryEvent::Verified { instance_id, record, .. } => {
                let ip = record.ip_address.expect("BUG: verified TEE record must have ip_address");
                let api_port = record.api_port.expect("BUG: verified TEE record must have api_port");

                let raft_port = common::config::CONSENSUS_API_PORT;
                let node_id = ip_to_node_id(&ip);
                let raft_addr = SocketAddr::new(IpAddr::V4(ip), raft_port);
                let api_addr = SocketAddr::new(IpAddr::V4(ip), api_port);

                TrustedPeerRegistry::get().mark_trusted(node_id, raft_addr, api_addr).await;

                info!(
                    instance_id = %instance_id,
                    node_id = node_id,
                    "TEE added to TrustedPeerRegistry"
                );
            }

            PeerRegistryEvent::Revoked { instance_id, reason, record, .. } => {
                let ip = record.ip_address.expect("BUG: revoked TEE record must have ip_address");
                let node_id = ip_to_node_id(&ip);

                TrustedPeerRegistry::get().revoke_trust(node_id).await;

                info!(
                    instance_id = %instance_id,
                    node_id = node_id,
                    reason = %reason,
                    "TEE removed from TrustedPeerRegistry"
                );
            }

            PeerRegistryEvent::Registered { .. } | PeerRegistryEvent::Rejected { .. } => {
                // No action needed - these are informational events
            }
        }
    }
}

/// Background task that periodically cleans up expired pending challenges.
pub async fn cleanup_task(registry: PeerRegistry) -> ! {
    let mut interval = tokio::time::interval(Duration::from_secs(CLEANUP_INTERVAL_SECS));

    loop {
        interval.tick().await;

        let expired = registry.cleanup_expired().await;
        if expired > 0 {
            debug!(count = expired, "Cleaned up expired pending challenges");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_ip_to_node_id_deterministic() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);

        let id1 = ip_to_node_id(&ip);
        let id2 = ip_to_node_id(&ip);

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_ip_to_node_id_different() {
        let ip1 = Ipv4Addr::new(192, 168, 1, 100);
        let ip2 = Ipv4Addr::new(192, 168, 1, 101);

        let id1 = ip_to_node_id(&ip1);
        let id2 = ip_to_node_id(&ip2);

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_register_request_serialization() {
        let request = RegisterRequest { evidence: "base64data".to_string() };

        let json = serde_json::to_string(&request).unwrap();
        let parsed: RegisterRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.evidence, "base64data");
    }

    #[test]
    fn test_verify_request_serialization() {
        let request = VerifyRequest { evidence: "base64data".to_string() };

        let json = serde_json::to_string(&request).unwrap();
        let parsed: VerifyRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.evidence, "base64data");
    }
}

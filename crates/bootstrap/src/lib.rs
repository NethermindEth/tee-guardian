// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Guardian bootstrap and peer trust management.
//!
//! This crate provides the trust and attestation infrastructure for the guardian service:
//!
//! - [`TrustedPeerRegistry`]: Thread-safe registry of attested peer node IDs
//! - [`AttestationApiState`]: State for the two-phase attestation protocol
//! - [`AttestationEventHandler`]: Background task handling PeerRegistry events
//! - [`PeerGossipManager`]: Peer-to-peer advertisement and discovery protocol
//! - [`GuardianNodeApiState`]: HTTP API state for peer communication
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Bootstrap Crate                          │
//! │                                                                  │
//! │  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
//! │  │ TrustedPeer     │◄─│ Attestation     │◄─│ PeerGossip      │ │
//! │  │ Registry        │  │ EventHandler    │  │ Manager         │ │
//! │  └────────┬────────┘  └────────┬────────┘  └─────────────────┘ │
//! │           │                    │                               │
//! │           │                    │ subscribes                    │
//! │           │                    ▼                               │
//! │           │          ┌─────────────────┐                       │
//! │           │          │  PeerRegistry   │ (from peer_registry)  │
//! │           │          │  (event source) │                       │
//! │           │          └────────┬────────┘                       │
//! │           │                   │                                │
//! │           │                   ▼                                │
//! │           │  ┌─────────────────────────────────────────────┐   │
//! │           │  │         GuardianNodeApi                      │   │
//! │           │  │  POST /attestation/{ns}/register             │   │
//! │           │  │  POST /attestation/{ns}/verify               │   │
//! │           │  │  GET  /guardian_node/peers                   │   │
//! │           │  │  POST /guardian_node/advertise               │   │
//! │           │  └─────────────────────────────────────────────┘   │
//! └───────────┼─────────────────────────────────────────────────────┘
//!             │
//!             ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       Consensus Crate                           │
//! │                                                                  │
//! │  Router checks TrustedPeerRegistry before each RPC              │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Two-Phase Attestation Protocol
//!
//! 1. Candidate POSTs initial quote to `/attestation/{ns}/register`
//! 2. Cluster extracts TeeId, verifies measurement, issues challenge
//! 3. Candidate POSTs challenge-bound quote to `/attestation/{ns}/verify`
//! 4. Cluster verifies, emits `TeeVerified` event
//! 5. `AttestationEventHandler` adds TEE to `TrustedPeerRegistry`

pub mod attestation;
pub mod gossip;
pub mod guardian_node_api;
pub mod metrics;
pub mod trusted_peers;

// Re-exports for convenience
pub use attestation::{
    cleanup_task, handle_register, handle_verify, AttestationApiState, AttestationError, AttestationEventHandler,
    RegisterRequest, VerifyRequest,
};
pub use gossip::{generate_gossip_evidence, GossipConfig, GossipedPeerCache, PeerAdvertisement, PeerGossipManager};
pub use guardian_node_api::{
    create_guardian_node_router, start_guardian_node_api, ApiError, GuardianNodeApiState, QuoteResponse,
};
pub use metrics::init_gossip_metrics;
pub use trusted_peers::TrustedPeerRegistry;

// Re-export commonly used types from peer_registry
pub use peer_registry::types::{ip_to_node_id, node_id_to_ip, BootstrapNode, NodeId};
pub use peer_registry::{
    InstanceId, MeasurementRegistryClient, PeerRegistry, PeerRegistryEvent, Tee, TeeRecord, TeeState, VerificationError,
};

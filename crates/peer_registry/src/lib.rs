// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Peer registry for TEE-based distributed systems.
//!
//! This crate provides platform-agnostic components for managing TEE attestation
//! and peer trust across TDX and SEV-SNP platforms.
//!
//! # Core Abstractions
//!
//! - **[`InstanceId`]**: Unique per-instance identity (from REPORTDATA or REPORT_ID)
//! - **[`TeeState`]**: Attestation lifecycle (PendingChallenge → Verified | Rejected)
//! - **[`TeeRecord`]**: Complete attestation metadata including state and network info
//! - **[`PeerRegistry`]**: Central registry with event broadcasting
//! - **[`PeerRegistryEvent`]**: State change notifications for subscribers
//!
//! # Backend Architecture
//!
//! Platform-specific verification is handled by the [`backend`] module:
//! - [`backend::TdxQuote`] / [`backend::VerifiedTdxQuote`] - Intel TDX (DCAP)
//! - [`backend::SnpReport`] / [`backend::VerifiedSnpReport`] - AMD SEV-SNP (VCEK/VLEK)
//!
//! # Two-Phase Authentication Protocol
//!
//! ```text
//! Candidate                                    Cluster
//!    │                                            │
//!    │ POST /attestation/{ns}/register            │
//!    │ { evidence }                               │
//!    │ ──────────────────────────────────────────>│
//!    │                                            │ Verify quote/report signature
//!    │                                            │ Extract instance_id from quote
//!    │                                            │ Verify measurement in whitelist
//!    │                                            │ Generate challenge
//!    │<────────────────────────────────────────── │
//!    │ { challenge, expires_at }                  │
//!    │                                            │
//!    │ POST /attestation/{ns}/verify              │
//!    │ { evidence }                               │
//!    │ ──────────────────────────────────────────>│
//!    │                                            │ Extract instance_id from quote
//!    │                                            │ Look up pending challenge
//!    │                                            │ Verify challenge in REPORTDATA[0:32]
//!    │                                            │ Emit Verified event
//!    │<────────────────────────────────────────── │
//!    │ { verified: true, ... }                    │
//! ```

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod backend;
pub mod measurement;
pub mod registry;
pub mod types;
pub mod verifier;

// Re-export identity types
pub use types::InstanceId;

// Re-export common byte types for convenience
pub use common::{Bytes32, Bytes48, Bytes64};

// Re-export TEE types
pub use types::{PeerRegistryEvent, TeeRecord, TeeState};

// Re-export kbs_types::Tee for platform identification
pub use kbs_types::Tee;

// Re-export registry
pub use registry::PeerRegistry;

// Re-export backend types
pub use backend::{
    detect_available_tee, detect_tee_type, generate_evidence, get_measurement_hash, is_tee_available,
    measurement_hash_from_evidence, measurement_hash_from_json, SnpReport, TdAttributes, TdxQuote, VerifiedSnpReport,
    VerifiedTdxQuote,
};

// Re-export error types
pub use types::{MeasurementRegistryError, PeerRegistryError, Result, VerificationError};

// Re-export measurement registry client
pub use measurement::MeasurementRegistryClient;

// Re-export types
pub use types::{
    ip_to_node_id, node_id_to_ip, BootstrapNode, NodeDiscoveryResponse, NodeId, ParsedAttestation,
    RegistrationResponse, TrustedMeasurement, TrustedMeasurementData, VerificationResponse,
};

// Re-export verifier protocol functions
pub use verifier::{register_tee, verify_tee};

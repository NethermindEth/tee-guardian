// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Type definitions for TEE attestation verification.
//!
//! This module contains all data structures used in attestation:
//! - [`ParsedAttestation`] - Platform-agnostic attestation result
//! - [`TrustedMeasurement`] - Registry-sourced trusted values
//! - [`RegistrationResponse`] / [`VerificationResponse`] - API responses
//! - [`InstanceId`] - Unique TEE instance identity
//! - [`TeeState`] / [`TeeRecord`] / [`PeerRegistryEvent`] - TEE state types
//! - Error types for peer registry operations
//! - Node and peer types for discovery

mod attestation;
pub mod error;
pub mod identity;
mod measurement;
mod node;
mod response;
pub mod tee;

pub use attestation::ParsedAttestation;
pub use error::{MeasurementRegistryError, PeerRegistryError, Result, VerificationError};
pub use identity::InstanceId;
pub use measurement::{
    CpuModel, SnpMeasurement, SnpMinimumTcb, SnpPlatformConstraints, TdxMeasurement, TdxMinimumTcb,
    TdxPlatformConstraints, TrustedMeasurement, TrustedMeasurementData,
};
pub use node::{
    ip_to_node_id, node_id_to_ip, BlacklistReason, BootstrapNode, NodeDiscoveryResponse, NodeId, PeerState,
    RevokedReason,
};
pub use response::{RegistrationResponse, VerificationResponse};
pub use tee::{PeerRegistryEvent, TeeRecord, TeeState};

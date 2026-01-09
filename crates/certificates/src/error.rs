// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Certificate error types

use thiserror::Error;

/// Result type for certificate operations
pub type CertificateResult<T> = Result<T, CertificateError>;

/// Errors that can occur during certificate operations
#[derive(Debug, Error)]
pub enum CertificateError {
    /// The requested namespace was not found
    #[error("Namespace not found: {0}")]
    NamespaceNotFound(String),

    /// The requested certificate was not found
    #[error("Certificate not found: serial {0}")]
    CertificateNotFound(u64),

    /// Attempted to use a reserved namespace
    #[error("Namespace '{0}' is reserved and cannot be used")]
    ReservedNamespace(String),

    /// Certificate has been revoked
    #[error("Certificate has been revoked: serial {serial}, reason: {reason}")]
    CertificateRevoked { serial: u64, reason: String },

    /// Certificate has expired
    #[error("Certificate has expired: serial {serial}")]
    CertificateExpired { serial: u64 },

    /// Invalid certificate request
    #[error("Invalid certificate request: {0}")]
    InvalidRequest(String),

    /// Attestation verification failed
    #[error("Attestation verification failed: {0}")]
    AttestationFailed(String),

    /// Measurement not in registry whitelist
    #[error("Measurement not trusted for namespace '{namespace}': {measurement_hash}")]
    MeasurementNotTrusted { namespace: String, measurement_hash: String },

    /// Storage error
    #[error("Storage error: {0}")]
    Storage(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

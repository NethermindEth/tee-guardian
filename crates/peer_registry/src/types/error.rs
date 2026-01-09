// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Error types for peer registry operations.

use thiserror::Error;

/// Result type for peer registry operations.
pub type Result<T> = std::result::Result<T, PeerRegistryError>;

/// Top-level error type for peer registry operations.
#[derive(Debug, Error)]
pub enum PeerRegistryError {
    /// Error during peer verification
    #[error("Verification error: {0}")]
    Verification(#[from] VerificationError),

    /// Error communicating with measurement registry
    #[error("Measurement registry error: {0}")]
    MeasurementRegistry(#[from] MeasurementRegistryError),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

/// Error types for peer verification operations.
#[derive(Debug, Error)]
pub enum VerificationError {
    // === Transient errors (retry with backoff) ===
    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Request timeout
    #[error("Request timeout: {0}")]
    Timeout(String),

    /// HTTP error from peer
    #[error("HTTP error {status}: {message}")]
    HttpError {
        /// HTTP status code
        status: u16,
        /// Error message
        message: String,
    },

    // === Parse/format errors (400) ===
    /// Failed to parse response
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Quote parsing failed
    #[error("Invalid quote format: {0}")]
    InvalidQuoteFormat(String),

    // === Attestation failures (400) ===
    /// Cryptographic verification failed
    #[error("Attestation failed: {0}")]
    AttestationFailed(String),

    // === Policy violations (403) ===
    /// Measurement not in registry whitelist
    #[error("Measurement not trusted: {0}")]
    MeasurementNotTrusted(String),

    /// Debug mode enabled but not allowed
    #[error("Debug mode enabled but not allowed by policy")]
    DebugNotAllowed,

    /// TCB version below minimum
    #[error("TCB version outdated: {0}")]
    TcbOutdated(String),

    /// Platform constraint violation
    #[error("Platform constraint violation: {0}")]
    PlatformConstraint(String),

    /// Measurement hash mismatch
    #[error("Measurement mismatch: expected '{expected}', got '{actual}'")]
    MeasurementMismatch {
        /// Expected measurement hash
        expected: String,
        /// Actual measurement hash
        actual: String,
    },

    // === Authentication errors (401) ===
    /// Challenge nonce mismatch
    #[error("Nonce verification failed: {0}")]
    NonceVerificationFailed(String),

    // === Not found (404) ===
    /// No pending challenge found
    #[error("No pending challenge for instance")]
    NoPendingChallenge,

    // === Conflict errors (409) ===
    /// TDX instance missing identity
    #[error("Identity required: TDX instances must provide non-zero REPORTDATA[32:64]")]
    IdentityRequired,

    /// Instance ID collision
    #[error("Identity conflict: instance {0} is already registered")]
    IdentityConflict(String),
}

impl VerificationError {
    /// Returns true if this error is transient and should be retried.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::ConnectionFailed(_) | Self::Timeout(_) | Self::HttpError { .. })
    }

    /// Returns the appropriate HTTP status code for this error.
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::ConnectionFailed(_) | Self::Timeout(_) => 503,
            Self::HttpError { status, .. } => *status,
            Self::ParseError(_) | Self::AttestationFailed(_) | Self::InvalidQuoteFormat(_) => 400,
            Self::NonceVerificationFailed(_) => 401,
            Self::MeasurementNotTrusted(_)
            | Self::DebugNotAllowed
            | Self::TcbOutdated(_)
            | Self::PlatformConstraint(_)
            | Self::MeasurementMismatch { .. } => 403,
            Self::NoPendingChallenge => 404,
            Self::IdentityRequired | Self::IdentityConflict(_) => 409,
        }
    }
}

/// Error types for measurement registry operations.
#[derive(Debug, Error)]
pub enum MeasurementRegistryError {
    /// Failed to connect to the registry service.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Request timed out.
    #[error("Request timeout: {0}")]
    Timeout(String),

    /// Registry returned an error response.
    #[error("Registry error (status {status}): {message}")]
    RegistryError {
        /// HTTP status code
        status: u16,
        /// Error message
        message: String,
    },

    /// Failed to parse registry response.
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Invalid URL configuration.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_codes() {
        assert_eq!(VerificationError::AttestationFailed("x".into()).status_code(), 400);
        assert_eq!(VerificationError::NonceVerificationFailed("x".into()).status_code(), 401);
        assert_eq!(VerificationError::MeasurementNotTrusted("x".into()).status_code(), 403);
        assert_eq!(VerificationError::DebugNotAllowed.status_code(), 403);
        assert_eq!(VerificationError::TcbOutdated("x".into()).status_code(), 403);
        assert_eq!(VerificationError::NoPendingChallenge.status_code(), 404);
        assert_eq!(VerificationError::IdentityRequired.status_code(), 409);
    }

    #[test]
    fn test_is_transient() {
        assert!(VerificationError::ConnectionFailed("x".into()).is_transient());
        assert!(VerificationError::Timeout("x".into()).is_transient());
        assert!(!VerificationError::AttestationFailed("x".into()).is_transient());
        assert!(!VerificationError::DebugNotAllowed.is_transient());
    }
}

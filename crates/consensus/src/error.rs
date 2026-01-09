// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Error types for consensus module.
//!
//! This module contains error types for RAFT operations only.
//! Peer verification errors are defined in the `peer_registry` crate.

use thiserror::Error;

/// Result type for consensus operations.
pub type ConsensusResult<T> = Result<T, ConsensusError>;

/// Consensus error types.
#[derive(Debug, Error)]
pub enum ConsensusError {
    /// Storage error
    #[error("Storage error: {0}")]
    Storage(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(String),

    /// Invalid join request
    #[error("Invalid join request: {0}")]
    InvalidJoinRequest(String),

    /// Node not found
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consensus_error_display() {
        let err = ConsensusError::Storage("disk full".to_string());
        assert_eq!(err.to_string(), "Storage error: disk full");
    }

    #[test]
    fn test_consensus_error_network() {
        let err = ConsensusError::Network("connection refused".to_string());
        assert_eq!(err.to_string(), "Network error: connection refused");
    }

    #[test]
    fn test_consensus_error_config() {
        let err = ConsensusError::Config("invalid port".to_string());
        assert_eq!(err.to_string(), "Configuration error: invalid port");
    }
}

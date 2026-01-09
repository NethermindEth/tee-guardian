// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE attestation types for multi-platform verification.
//!
//! This module provides platform-independent abstractions for:
//! - Instance identity ([`InstanceId`]) - unique per TEE instance
//! - TEE state machine ([`TeeState`]) - pending, verified, rejected
//! - TEE records ([`TeeRecord`]) - full attestation metadata
//!
//! # Design Philosophy
//!
//! TEE instance identity is platform-specific:
//! - **TDX**: Client generates 32 random bytes in `REPORTDATA[32:64]`
//! - **SEV-SNP**: Firmware generates `REPORT_ID` (32 bytes)
//!
//! The `InstanceId` is used as the primary key for tracking attestation sessions
//! and looking up verified TEEs.
//!
//! # Supported Platforms
//!
//! Uses `kbs_types::Tee` for platform identification (TDX, SEV-SNP, etc.)

use chrono::{DateTime, Utc};
use kbs_types::Tee;
use std::net::Ipv4Addr;
use std::time::Instant;

use super::identity::InstanceId;

/// Attestation session state.
#[derive(Debug, Clone)]
pub enum TeeState {
    /// Challenge issued, awaiting verification (Phase 1 complete).
    PendingChallenge {
        /// Random challenge nonce (32 bytes)
        challenge: [u8; 32],
        /// When the challenge was issued
        issued_at: Instant,
        /// Challenge expiration time
        expires_at: Instant,
        /// Pre-verified measurement hash (from Phase 1)
        measurement_hash: String,
        /// TEE platform type
        tee: Tee,
    },

    /// Successfully verified and active.
    Verified {
        /// SHA-256 hash of RTMRs/measurements
        measurement_hash: String,
        /// TCB status from attestation verification
        tcb_status: String,
        /// TEE platform type
        tee: Tee,
        /// When verification completed
        verified_at: DateTime<Utc>,
    },

    /// Verification failed or trust revoked.
    Rejected {
        /// Reason for rejection
        reason: String,
        /// When rejection occurred
        rejected_at: DateTime<Utc>,
    },
}

impl TeeState {
    /// Returns true if verified and active.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    /// Returns true if awaiting challenge verification.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::PendingChallenge { .. })
    }

    /// Returns true if rejected.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }

    /// Get the measurement hash if available.
    #[must_use]
    pub fn measurement_hash(&self) -> Option<&str> {
        match self {
            Self::Verified { measurement_hash, .. } | Self::PendingChallenge { measurement_hash, .. } => {
                Some(measurement_hash)
            }
            Self::Rejected { .. } => None,
        }
    }

    /// Get the TEE platform type if available.
    #[must_use]
    pub fn tee(&self) -> Option<Tee> {
        match self {
            Self::Verified { tee, .. } | Self::PendingChallenge { tee, .. } => Some(*tee),
            Self::Rejected { .. } => None,
        }
    }
}

/// Attestation session record.
///
/// Tracks a single attestation attempt from Phase 1 (registration) through
/// Phase 2 (verification).
#[derive(Debug, Clone)]
pub struct TeeRecord {
    /// Instance identity (from REPORTDATA[32:64] for TDX, REPORT_ID for SEV-SNP)
    pub instance_id: InstanceId,
    /// Namespace this attestation is for
    pub namespace: String,
    /// Current state
    pub state: TeeState,
    /// Network address (if known)
    pub ip_address: Option<Ipv4Addr>,
    /// API port (if known)
    pub api_port: Option<u16>,
}

impl TeeRecord {
    /// Create a new record in pending state.
    #[must_use]
    pub fn new_pending(
        instance_id: InstanceId,
        namespace: String,
        challenge: [u8; 32],
        measurement_hash: String,
        tee: Tee,
        expires_in: std::time::Duration,
    ) -> Self {
        let now = Instant::now();
        Self {
            instance_id,
            namespace,
            state: TeeState::PendingChallenge {
                challenge,
                issued_at: now,
                expires_at: now + expires_in,
                measurement_hash,
                tee,
            },
            ip_address: None,
            api_port: None,
        }
    }

    /// Check if the challenge has expired.
    #[must_use]
    pub fn is_challenge_expired(&self) -> bool {
        match &self.state {
            TeeState::PendingChallenge { expires_at, .. } => Instant::now() > *expires_at,
            _ => false,
        }
    }

    /// Get the challenge if pending and not expired.
    #[must_use]
    pub fn get_valid_challenge(&self) -> Option<&[u8; 32]> {
        match &self.state {
            TeeState::PendingChallenge { challenge, expires_at, .. } => {
                if Instant::now() <= *expires_at {
                    Some(challenge)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Events emitted by the PeerRegistry on state changes.
#[derive(Debug, Clone)]
pub enum PeerRegistryEvent {
    /// Attestation registered and challenge issued (Phase 1 complete).
    Registered {
        /// Namespace
        namespace: String,
        /// Instance ID for this TEE
        instance_id: InstanceId,
        /// Challenge nonce issued
        challenge: [u8; 32],
        /// Measurement hash (pre-verified in Phase 1)
        measurement_hash: String,
    },

    /// Attestation successfully verified (Phase 2 complete).
    Verified {
        /// Namespace
        namespace: String,
        /// Instance ID
        instance_id: InstanceId,
        /// Full attestation record
        record: TeeRecord,
    },

    /// Attestation verification failed.
    Rejected {
        /// Namespace
        namespace: String,
        /// Instance ID
        instance_id: InstanceId,
        /// Rejection reason
        reason: String,
    },

    /// Previously verified attestation had trust revoked.
    Revoked {
        /// Namespace
        namespace: String,
        /// Instance ID
        instance_id: InstanceId,
        /// Revocation reason
        reason: String,
        /// Full attestation record (for cleanup)
        record: TeeRecord,
    },
}

impl PeerRegistryEvent {
    /// Get the namespace for this event.
    #[must_use]
    pub fn namespace(&self) -> &str {
        match self {
            Self::Registered { namespace, .. }
            | Self::Verified { namespace, .. }
            | Self::Rejected { namespace, .. }
            | Self::Revoked { namespace, .. } => namespace,
        }
    }

    /// Get the instance ID for this event.
    #[must_use]
    pub fn instance_id(&self) -> &InstanceId {
        match self {
            Self::Registered { instance_id, .. }
            | Self::Verified { instance_id, .. }
            | Self::Rejected { instance_id, .. }
            | Self::Revoked { instance_id, .. } => instance_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_tee_state_is_verified() {
        let verified = TeeState::Verified {
            measurement_hash: "abc123".to_string(),
            tcb_status: "UpToDate".to_string(),
            tee: Tee::Tdx,
            verified_at: Utc::now(),
        };

        assert!(verified.is_verified());
        assert!(!verified.is_pending());
        assert!(!verified.is_rejected());
        assert_eq!(verified.tee(), Some(Tee::Tdx));
    }

    #[test]
    fn test_tee_record_pending() {
        let instance_id = InstanceId::from_bytes([0xAB; 32]);
        let record = TeeRecord::new_pending(
            instance_id,
            "guardian".to_string(),
            [0xAA; 32],
            "abc123".to_string(),
            Tee::Tdx,
            Duration::from_secs(60),
        );

        assert!(record.state.is_pending());
        assert!(!record.is_challenge_expired());
        assert!(record.get_valid_challenge().is_some());
    }

    #[test]
    fn test_peer_registry_event_namespace() {
        let instance_id = InstanceId::from_bytes([0xAB; 32]);
        let event = PeerRegistryEvent::Verified {
            namespace: "guardian".to_string(),
            instance_id,
            record: TeeRecord::new_pending(
                instance_id,
                "guardian".to_string(),
                [0; 32],
                "hash".to_string(),
                Tee::Tdx,
                Duration::from_secs(60),
            ),
        };

        assert_eq!(event.namespace(), "guardian");
    }
}

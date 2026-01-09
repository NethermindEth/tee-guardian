// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Node and peer state types.

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::time::Instant;

/// Node ID is the IPv4 address packed into a u64.
///
/// # Example
/// ```text
/// 192.168.1.100 -> 0xC0A80164 -> 3232235876
/// ```
pub type NodeId = u64;

/// Convert an IPv4 address to a node ID.
#[must_use]
pub fn ip_to_node_id(ip: &Ipv4Addr) -> NodeId {
    ip.octets().into_iter().fold(0, |acc, octet| acc << 8 | u64::from(octet))
}

/// Convert a node ID back to an IPv4 address.
#[must_use]
pub fn node_id_to_ip(node_id: NodeId) -> Ipv4Addr {
    Ipv4Addr::new(
        ((node_id >> 24) & 0xFF) as u8,
        ((node_id >> 16) & 0xFF) as u8,
        ((node_id >> 8) & 0xFF) as u8,
        (node_id & 0xFF) as u8,
    )
}

/// Peer lifecycle state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerState {
    /// Peer discovered via registry or gossip, not yet verified.
    Discovered,
    /// Attestation verification is in progress.
    Verifying,
    /// Successfully attested, allowed to participate in consensus.
    Trusted,
    /// Attestation failed with a blacklist-worthy error.
    Blacklisted {
        /// When the blacklist expires
        until: Instant,
        /// Reason for blacklisting
        reason: BlacklistReason,
    },
    /// Was trusted, but has been revoked.
    Revoked {
        /// Reason for revocation
        reason: RevokedReason,
    },
}

impl PeerState {
    /// Check if this peer is currently trusted.
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Trusted)
    }

    /// Check if this peer is currently blacklisted.
    #[must_use]
    pub fn is_blacklisted(&self) -> bool {
        matches!(self, Self::Blacklisted { .. })
    }

    /// Check if the blacklist has expired.
    #[must_use]
    pub fn is_blacklist_expired(&self) -> bool {
        match self {
            Self::Blacklisted { until, .. } => Instant::now() >= *until,
            _ => false,
        }
    }
}

/// Reason for blacklisting a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlacklistReason {
    /// TDX quote signature verification failed
    AttestationFailed,
    /// Nonce in report_data didn't match challenge
    NonceVerificationFailed,
    /// Quote was malformed
    InvalidQuoteFormat,
}

impl std::fmt::Display for BlacklistReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AttestationFailed => write!(f, "attestation_failed"),
            Self::NonceVerificationFailed => write!(f, "nonce_verification_failed"),
            Self::InvalidQuoteFormat => write!(f, "invalid_quote_format"),
        }
    }
}

/// Reason for revoking a previously trusted peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevokedReason {
    /// Measurement was removed from the registry whitelist
    MeasurementRevoked,
    /// Manual revocation by administrator
    ManualRevocation,
    /// Re-attestation was required and failed
    ReattestationFailed,
    /// Custom reason
    Other(String),
}

impl std::fmt::Display for RevokedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MeasurementRevoked => write!(f, "measurement_revoked"),
            Self::ManualRevocation => write!(f, "manual_revocation"),
            Self::ReattestationFailed => write!(f, "reattestation_failed"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Bootstrap node information from measurement registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapNode {
    /// IPv4 address of the node
    pub ip_address: Ipv4Addr,
    /// RAFT consensus port
    pub raft_port: u16,
    /// API port for attestation endpoints
    pub api_port: u16,
}

impl BootstrapNode {
    /// Get the node ID derived from this node's IP address.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        ip_to_node_id(&self.ip_address)
    }
}

/// Node discovery response from measurement registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDiscoveryResponse {
    /// List of bootstrap nodes registered for the namespace
    pub nodes: Vec<BootstrapNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_to_node_id() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let node_id = ip_to_node_id(&ip);
        assert_eq!(node_id, 0xC0A8_0164);
    }

    #[test]
    fn test_node_id_to_ip() {
        let ip = node_id_to_ip(0xC0A8_0164);
        assert_eq!(ip, Ipv4Addr::new(192, 168, 1, 100));
    }

    #[test]
    fn test_roundtrip() {
        let original_ip = Ipv4Addr::new(10, 0, 0, 1);
        let node_id = ip_to_node_id(&original_ip);
        let recovered_ip = node_id_to_ip(node_id);
        assert_eq!(original_ip, recovered_ip);
    }

    #[test]
    fn test_peer_state_is_trusted() {
        assert!(PeerState::Trusted.is_trusted());
        assert!(!PeerState::Discovered.is_trusted());
    }

    #[test]
    fn test_bootstrap_node_node_id() {
        let node = BootstrapNode { ip_address: Ipv4Addr::new(192, 168, 1, 100), raft_port: 8444, api_port: 8443 };
        assert_eq!(node.node_id(), 0xC0A8_0164);
    }
}

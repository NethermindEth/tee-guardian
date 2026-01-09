// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Shared API types for Guardian service.
//!
//! These types are used across crates for health checks, cluster coordination,
//! and API responses.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Minimum trusted peers required for bootstrap consideration.
pub const MIN_PEERS_FOR_BOOTSTRAP: usize = 2;

/// Node lifecycle state in the RAFT cluster.
///
/// Serializes to SCREAMING_SNAKE_CASE for JSON API compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeStatus {
    /// Not yet in a cluster, waiting for mutual attestation.
    Bootstrapping,
    /// Added to cluster but not yet a voting member.
    Learner,
    /// Voting member, following the current leader.
    Follower,
    /// Voting member, running for leader election.
    Candidate,
    /// Current cluster leader.
    Leader,
}

impl NodeStatus {
    /// Check if this status indicates the node is a leader.
    #[inline]
    #[must_use]
    pub const fn is_leader(self) -> bool {
        matches!(self, Self::Leader)
    }

    /// Check if this status indicates the node is bootstrapping.
    #[inline]
    #[must_use]
    pub const fn is_bootstrapping(self) -> bool {
        matches!(self, Self::Bootstrapping)
    }

    /// Check if this status indicates the node is in an active cluster.
    #[inline]
    #[must_use]
    pub const fn in_cluster(self) -> bool {
        matches!(self, Self::Learner | Self::Follower | Self::Candidate | Self::Leader)
    }
}

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bootstrapping => write!(f, "BOOTSTRAPPING"),
            Self::Learner => write!(f, "LEARNER"),
            Self::Follower => write!(f, "FOLLOWER"),
            Self::Candidate => write!(f, "CANDIDATE"),
            Self::Leader => write!(f, "LEADER"),
        }
    }
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self::Bootstrapping
    }
}

/// Health check response with cluster state information.
///
/// This is a minimal response with no computed/redundant fields.
/// Clients should derive any additional state they need:
///
/// - **Is leader?** → `status.is_leader()`
/// - **Ready for bootstrap?** → `cluster_size == 0 && trusted_peers.len() >= 2 && status.is_bootstrapping()`
/// - **In a cluster?** → `cluster_size > 0`
///
/// # Example Response
///
/// ```json
/// {
///   "node_id": 3232266772,
///   "status": "FOLLOWER",
///   "term": 5,
///   "cluster_size": 3,
///   "trusted_peers": [3232266773, 3232266774]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// This node's ID (derived from IP address).
    pub node_id: u64,

    /// Node lifecycle state.
    pub status: NodeStatus,

    /// Current RAFT term (0 if bootstrapping).
    pub term: u64,

    /// Number of voters in RAFT cluster membership.
    pub cluster_size: usize,

    /// Node IDs that this node has attested and trusts.
    ///
    /// Used by RaftCoordinator for mutual readiness checks during bootstrap.
    #[serde(default)]
    pub trusted_peers: Vec<u64>,
}

impl HealthResponse {
    /// Check if this node is the leader.
    #[inline]
    #[must_use]
    pub const fn is_leader(&self) -> bool {
        self.status.is_leader()
    }

    /// Check if this node is bootstrapping (not yet in a cluster).
    #[inline]
    #[must_use]
    pub const fn is_bootstrapping(&self) -> bool {
        self.status.is_bootstrapping()
    }

    /// Check if this node is in an active cluster.
    #[inline]
    #[must_use]
    pub const fn in_cluster(&self) -> bool {
        self.cluster_size > 0
    }

    /// Check if this node is ready to participate in cluster bootstrap.
    ///
    /// A node is ready when:
    /// 1. Not already in a cluster (`cluster_size == 0`)
    /// 2. Has enough trusted peers (`trusted_peers.len() >= min_peers`)
    /// 3. Is in bootstrapping state
    #[inline]
    #[must_use]
    pub fn ready_for_bootstrap(&self, min_peers: usize) -> bool {
        self.cluster_size == 0 && self.trusted_peers.len() >= min_peers && self.status.is_bootstrapping()
    }

    /// Check if this peer is ready and trusts the given node_id.
    ///
    /// Used for mutual readiness checks during bootstrap coordination.
    #[inline]
    #[must_use]
    pub fn is_mutually_ready(&self, our_node_id: u64, min_peers: usize) -> bool {
        self.ready_for_bootstrap(min_peers) && self.trusted_peers.contains(&our_node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_status_display() {
        assert_eq!(NodeStatus::Bootstrapping.to_string(), "BOOTSTRAPPING");
        assert_eq!(NodeStatus::Learner.to_string(), "LEARNER");
        assert_eq!(NodeStatus::Follower.to_string(), "FOLLOWER");
        assert_eq!(NodeStatus::Candidate.to_string(), "CANDIDATE");
        assert_eq!(NodeStatus::Leader.to_string(), "LEADER");
    }

    #[test]
    fn test_node_status_methods() {
        assert!(NodeStatus::Leader.is_leader());
        assert!(!NodeStatus::Follower.is_leader());

        assert!(NodeStatus::Bootstrapping.is_bootstrapping());
        assert!(!NodeStatus::Leader.is_bootstrapping());

        assert!(NodeStatus::Leader.in_cluster());
        assert!(NodeStatus::Follower.in_cluster());
        assert!(NodeStatus::Learner.in_cluster());
        assert!(!NodeStatus::Bootstrapping.in_cluster());
    }

    #[test]
    fn test_health_response_is_leader() {
        let health = HealthResponse {
            node_id: 123,
            status: NodeStatus::Leader,
            term: 5,
            cluster_size: 3,
            trusted_peers: vec![456, 789],
        };
        assert!(health.is_leader());
        assert!(!health.is_bootstrapping());
        assert!(health.in_cluster());
    }

    #[test]
    fn test_health_response_is_bootstrapping() {
        let health = HealthResponse {
            node_id: 123,
            status: NodeStatus::Bootstrapping,
            term: 0,
            cluster_size: 0,
            trusted_peers: vec![456, 789],
        };
        assert!(!health.is_leader());
        assert!(health.is_bootstrapping());
        assert!(!health.in_cluster());
    }

    #[test]
    fn test_health_response_ready_for_bootstrap() {
        // Ready: bootstrapping, no cluster, enough peers
        let ready = HealthResponse {
            node_id: 123,
            status: NodeStatus::Bootstrapping,
            term: 0,
            cluster_size: 0,
            trusted_peers: vec![456, 789],
        };
        assert!(ready.ready_for_bootstrap(2));
        assert!(!ready.ready_for_bootstrap(3)); // Not enough peers

        // Not ready: already in cluster
        let in_cluster = HealthResponse {
            node_id: 123,
            status: NodeStatus::Follower,
            term: 5,
            cluster_size: 3,
            trusted_peers: vec![456, 789],
        };
        assert!(!in_cluster.ready_for_bootstrap(2));

        // Not ready: not enough peers
        let not_enough = HealthResponse {
            node_id: 123,
            status: NodeStatus::Bootstrapping,
            term: 0,
            cluster_size: 0,
            trusted_peers: vec![456],
        };
        assert!(!not_enough.ready_for_bootstrap(2));
    }

    #[test]
    fn test_health_response_is_mutually_ready() {
        let health = HealthResponse {
            node_id: 123,
            status: NodeStatus::Bootstrapping,
            term: 0,
            cluster_size: 0,
            trusted_peers: vec![456, 789],
        };

        // Peer trusts node 456
        assert!(health.is_mutually_ready(456, 2));

        // Peer doesn't trust node 999
        assert!(!health.is_mutually_ready(999, 2));

        // Not enough peers
        assert!(!health.is_mutually_ready(456, 3));
    }

    #[test]
    fn test_health_response_deserialize() {
        let json = r#"{
            "node_id": 3232266772,
            "status": "BOOTSTRAPPING",
            "term": 0,
            "cluster_size": 0,
            "trusted_peers": [3232266773, 3232266774]
        }"#;

        let health: HealthResponse = serde_json::from_str(json).unwrap();
        assert_eq!(health.node_id, 3232266772);
        assert_eq!(health.status, NodeStatus::Bootstrapping);
        assert_eq!(health.term, 0);
        assert_eq!(health.cluster_size, 0);
        assert_eq!(health.trusted_peers, vec![3232266773, 3232266774]);
    }

    #[test]
    fn test_health_response_serialize() {
        let health = HealthResponse {
            node_id: 123,
            status: NodeStatus::Leader,
            term: 5,
            cluster_size: 3,
            trusted_peers: vec![456],
        };
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains(r#""status":"LEADER""#));
    }

    #[test]
    fn test_health_response_deserialize_minimal() {
        // Test backward compatibility - trusted_peers defaults to empty
        let json = r#"{
            "node_id": 3232266772,
            "status": "BOOTSTRAPPING",
            "term": 0,
            "cluster_size": 0
        }"#;

        let health: HealthResponse = serde_json::from_str(json).unwrap();
        assert!(health.trusted_peers.is_empty());
    }
}

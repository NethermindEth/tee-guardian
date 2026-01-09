// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! RAFT consensus node with trust-gated networking.
//!
//! # Security Model
//!
//! The RAFT layer is intentionally **attestation-agnostic**. It relies on:
//!
//! 1. [`GuardianTrustedPeerSet`] trait - implemented by the bootstrap crate
//! 2. [`Router`](crate::network::Router) - blocks RPC to untrusted peers
//!
//! This separation means:
//! - RAFT doesn't verify attestations directly
//! - Untrusted peers simply appear as "unreachable" to OpenRaft
//! - OpenRaft handles unreachable peers automatically (retries, leader election, etc.)
//!
//! # Cluster Formation
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │              Bootstrap Crate (PeerAttestationManager)           │
//! │  - Discovers peers via measurement registry + gossip            │
//! │  - Performs TDX attestation verification                        │
//! │  - Populates TrustedPeerRegistry on success                     │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │              TrustedPeerRegistry (GuardianTrustedPeerSet)        │
//! │  - Set of attested node IDs                                     │
//! │  - Shared by Router and bootstrap crate                         │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         Router                                   │
//! │  - Calls is_trusted() before each RPC                           │
//! │  - Returns Unreachable for untrusted peers                      │
//! │  - OpenRaft handles unreachable peers automatically             │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        RaftNode                                  │
//! │  - Pure RAFT consensus (leader election, log replication)       │
//! │  - No attestation logic                                         │
//! │  - Membership changes via add_learner() + change_membership()   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use crate::error::{ConsensusError, ConsensusResult};
use crate::network::Router;
use crate::types::{ClusterConfig, NodeId, Raft};
use openraft::Config;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

/// RAFT consensus node with trust-gated networking.
///
/// # Responsibilities
///
/// - **Consensus**: Leader election and log replication via OpenRaft
/// - **Membership**: Adding/removing nodes from the cluster
/// - **Network coordination**: Manages peer addresses via Router
///
/// # What This Does NOT Do
///
/// - **Attestation verification**: Handled by `PeerAttestationManager`
/// - **Trust decisions**: Managed by `TrustedPeerRegistry`
/// - **Join request handling**: Removed - nodes join when mutually attested
///
/// # Lifecycle
///
/// 1. `new()` - Create RAFT instance with Router
/// 2. `initialize_cluster()` - Bootstrap with mutually-attested peers
/// 3. Consensus operations (automatic via OpenRaft)
/// 4. `add_peer()` / `remove_peer()` - Called by `RaftCoordinator` when peers attest
/// 5. `shutdown()` - Graceful termination
pub struct RaftNode {
    /// Unique identifier within the cluster (derived from IP address)
    node_id: NodeId,

    /// Static configuration (addresses, registry URL, namespace)
    #[allow(dead_code)]
    config: ClusterConfig,

    /// OpenRaft instance - handles consensus protocol
    raft: Raft,

    /// Network layer for RPC (cheap to clone, internally Arc'd)
    router: Router,

    /// Persistent storage backend
    #[allow(dead_code)]
    storage: crate::storage::Store,
}

impl RaftNode {
    /// Create a new RAFT node (not yet part of any cluster).
    ///
    /// Uses the global `TrustedPeerRegistry` singleton for trust-gated networking.
    /// The Router will block RPC to any peer not in the registry.
    ///
    /// # Panics
    ///
    /// Panics if `TrustedPeerRegistry::init()` was not called first.
    ///
    /// # Arguments
    ///
    /// * `config` - Static cluster configuration (addresses, registry URL, etc.)
    /// * `storage_path` - Path for persistent RAFT state storage
    pub async fn new(config: ClusterConfig, storage_path: PathBuf) -> ConsensusResult<Self> {
        info!("Initializing RAFT node {}", config.node_id);

        let storage = crate::storage::new_store(&storage_path)
            .await
            .map_err(|e| ConsensusError::Storage(format!("Failed to open storage at {:?}: {}", storage_path, e)))?;
        let router = Router::new();

        let raft_config = Arc::new(
            Config {
                heartbeat_interval: 500,
                election_timeout_min: 1500,
                election_timeout_max: 3000,
                ..Default::default()
            }
            .validate()
            .map_err(|e| ConsensusError::Config(format!("Invalid RAFT config: {}", e)))?,
        );

        let raft = openraft::Raft::new(config.node_id, raft_config, router.clone(), storage.clone(), storage.clone())
            .await
            .map_err(|e| ConsensusError::Other(format!("Failed to create RAFT instance: {}", e)))?;

        info!("RAFT node {} initialized", config.node_id);

        Ok(Self { node_id: config.node_id, config, raft, router, storage })
    }

    /// Bootstrap a new cluster with mutually-attested peers.
    ///
    /// # Prerequisites
    ///
    /// - All peers must have completed mutual TDX attestation
    /// - All peers must be in the shared TrustedPeerRegistry
    /// - All peers should call this method ~simultaneously
    ///
    /// # Behavior
    ///
    /// 1. Adds all trusted peer addresses to the Router
    /// 2. Calls OpenRaft `initialize()` with full membership
    /// 3. OpenRaft automatically elects a leader
    pub async fn initialize_cluster(
        &self,
        trusted_peer_addresses: BTreeMap<NodeId, std::net::SocketAddr>,
    ) -> ConsensusResult<()> {
        info!(
            node_id = self.node_id,
            peer_count = trusted_peer_addresses.len(),
            "Initializing RAFT cluster with trusted peers"
        );

        let mut members = BTreeMap::new();

        for (peer_id, peer_addr) in trusted_peer_addresses {
            self.router.add_node(peer_id, peer_addr).await;
            members.insert(peer_id, openraft::BasicNode { addr: peer_addr.to_string() });
        }

        // Add self
        members.insert(self.node_id, openraft::BasicNode { addr: self.config.raft_address.to_string() });

        info!(node_id = self.node_id, cluster_size = members.len(), "Initializing RAFT with member set");

        self.raft
            .initialize(members)
            .await
            .map_err(|e| ConsensusError::Other(format!("Failed to initialize cluster: {}", e)))?;

        info!(node_id = self.node_id, "RAFT cluster initialized, awaiting leader election");

        Ok(())
    }

    /// Add a newly-attested peer to the cluster.
    ///
    /// Called by `RaftCoordinator` when a peer is newly attested. This is a
    /// leader-only operation - followers will reject with ForwardToLeader.
    ///
    /// The peer must already be in the `TrustedPeerRegistry` (done by
    /// `PeerAttestationManager` before calling this).
    pub async fn add_peer(&self, peer_id: NodeId, peer_addr: std::net::SocketAddr) -> ConsensusResult<()> {
        info!(peer_id = peer_id, peer_addr = %peer_addr, "Adding peer to cluster");

        // Add to router so RAFT can communicate
        self.router.add_node(peer_id, peer_addr).await;

        let node = openraft::BasicNode { addr: peer_addr.to_string() };

        // Add as learner first (non-voting, receives log replication)
        self.raft
            .add_learner(peer_id, node, true)
            .await
            .map_err(|e| ConsensusError::Other(format!("Failed to add learner: {}", e)))?;

        info!(peer_id = peer_id, "Peer added as learner");

        // Promote to voter
        let metrics = self.raft.metrics().borrow().clone();
        let current_membership = metrics.membership_config.membership();
        let mut new_voters: Vec<NodeId> = current_membership.voter_ids().collect();
        if !new_voters.contains(&peer_id) {
            new_voters.push(peer_id);
        }

        self.raft
            .change_membership(new_voters, false)
            .await
            .map_err(|e| ConsensusError::Other(format!("Failed to promote to voter: {}", e)))?;

        info!(peer_id = peer_id, "Peer promoted to voter");
        Ok(())
    }

    /// Remove a peer from the cluster.
    ///
    /// Called when a peer's trust is revoked or it becomes permanently unreachable.
    pub async fn remove_peer(&self, peer_id: NodeId) -> ConsensusResult<()> {
        info!(peer_id = peer_id, "Removing peer from cluster");

        // Remove from voter set
        let metrics = self.raft.metrics().borrow().clone();
        let current_membership = metrics.membership_config.membership();
        let new_voters: Vec<NodeId> = current_membership.voter_ids().filter(|&id| id != peer_id).collect();

        if new_voters.len() < current_membership.voter_ids().count() {
            self.raft
                .change_membership(new_voters, false)
                .await
                .map_err(|e| ConsensusError::Other(format!("Failed to remove voter: {}", e)))?;
        }

        // Remove from router
        self.router.remove_node(peer_id).await;

        info!(peer_id = peer_id, "Peer removed from cluster");
        Ok(())
    }

    /// Check if this node is the leader
    pub async fn is_leader(&self) -> bool {
        let metrics = self.raft.metrics().borrow().clone();
        metrics.current_leader == Some(self.node_id)
    }

    /// Get current leader ID (if known)
    pub fn current_leader(&self) -> Option<NodeId> {
        self.raft.metrics().borrow().current_leader
    }

    /// Get node ID
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Get node state for health checks
    pub async fn get_node_state(&self) -> ConsensusResult<crate::types::NodeState> {
        let metrics = self.raft.metrics().borrow().clone();

        let is_leader = metrics.current_leader == Some(self.node_id);
        let membership = metrics.membership_config.membership();
        let voters: Vec<NodeId> = membership.voter_ids().collect();
        let learners: Vec<NodeId> = membership.learner_ids().collect();

        let status = if learners.contains(&self.node_id) {
            common::NodeStatus::Learner
        } else if voters.is_empty() && learners.is_empty() {
            common::NodeStatus::Bootstrapping
        } else if is_leader {
            common::NodeStatus::Leader
        } else if metrics.current_leader.is_some() {
            common::NodeStatus::Follower
        } else {
            common::NodeStatus::Candidate
        };

        Ok(crate::types::NodeState { status, term: metrics.current_term })
    }

    /// Get RAFT instance (for external RPC handling)
    pub fn raft(&self) -> &Raft {
        &self.raft
    }

    /// Get router (for external RPC handling)
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// Shutdown node gracefully
    pub async fn shutdown(&self) -> ConsensusResult<()> {
        info!("Shutting down RAFT node {}", self.node_id);

        self.raft.shutdown().await.map_err(|e| ConsensusError::Other(format!("Shutdown error: {}", e)))?;

        info!("Node {} shutdown complete", self.node_id);
        Ok(())
    }
}

// Note: RaftNode tests require the global TrustedPeerRegistry singleton to be
// initialized. These tests are in integration tests (tests/test_consensus.rs).

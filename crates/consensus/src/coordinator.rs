// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! RAFT cluster formation coordinator.
//!
//! The [`RaftCoordinator`] manages cluster bootstrap with:
//! - Quorum-aware bootstrap decisions
//! - Mutual readiness verification (prevents split-brain)
//! - Automatic failover if designated bootstrap node fails
//! - New peer addition once cluster is active
//!
//! # State Machine
//!
//! ```text
//!                    ┌──────────────────┐
//!                    │      START       │
//!                    └────────┬─────────┘
//!                             │
//!                             ▼
//!               ┌─────────────────────────┐
//!               │   WAITING_FOR_PEERS     │◄────────────┐
//!               │  (trusted_count >= 2)   │             │
//!               └────────────┬────────────┘             │
//!                            │                          │
//!                            ▼                          │
//!               ┌─────────────────────────┐             │
//!               │   CHECKING_CLUSTER      │             │
//!               └────────────┬────────────┘             │
//!                            │                          │
//!          ┌─────────────────┴─────────────────┐        │
//!          │                                   │        │
//!   cluster exists                      no cluster      │
//!          │                                   │        │
//!          ▼                                   ▼        │
//! ┌─────────────────┐          ┌─────────────────────┐  │
//! │    JOINING      │          │ COORDINATING_BOOT   │  │
//! │ (wait for add)  │          │  (mutual ready)     │  │
//! └────────┬────────┘          └──────────┬──────────┘  │
//!          │                              │             │
//!          │              ┌───────────────┴──────┐      │
//!          │              │                      │      │
//!          │         I am lowest            not lowest  │
//!          │              │                      │      │
//!          │              ▼                      ▼      │
//!          │      ┌─────────────┐    ┌────────────────┐ │
//!          │      │ BOOTSTRAPPING│    │WAITING_BOOTSTRAP│
//!          │      └──────┬──────┘    └───────┬────────┘ │
//!          │             │                   │          │
//!          │             │    cluster formed │          │
//!          │             │    OR failover    │          │
//!          │             ▼                   ▼          │
//!          │      ┌─────────────────────────────┐       │
//!          └─────▶│       CLUSTER_ACTIVE        │◄──────┘
//!                 └─────────────────────────────┘
//! ```
//!
//! # Mutual Readiness Protocol
//!
//! A node is considered "ready" for bootstrap only when:
//! 1. It has attested sufficient peers (`trusted_peers.len() >= MIN_PEERS`)
//! 2. It is not already in a cluster (`cluster_size == 0`)
//! 3. It responds to health checks (is reachable)
//! 4. **It trusts us** (mutual trust verification)
//!
//! The node with the **lowest node_id** among mutually-ready peers bootstraps.

use crate::raft_node::RaftNode;
use crate::types::NodeId;
use bootstrap::TrustedPeerRegistry;
use common::{HealthResponse, MIN_PEERS_FOR_BOOTSTRAP};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Minimum mutually-ready nodes required to bootstrap (includes self).
const BOOTSTRAP_QUORUM: usize = 3;

/// Polling interval during discovery phases.
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(2);

/// Timeout waiting for designated bootstrap node.
const BOOTSTRAP_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout waiting to be added to existing cluster.
const MEMBERSHIP_WAIT_TIMEOUT: Duration = Duration::from_secs(300);

/// Interval for checking if new peers should be added (leader only).
const PEER_MONITOR_INTERVAL: Duration = Duration::from_secs(5);

/// HTTP client timeout for health queries.
const HEALTH_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Internal state of the coordinator state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CoordinatorState {
    /// Waiting for sufficient trusted peers.
    WaitingForPeers,
    /// Checking if a cluster already exists.
    CheckingCluster,
    /// Building mutual readiness set and coordinating bootstrap.
    CoordinatingBootstrap,
    /// This node will bootstrap the cluster.
    Bootstrapping,
    /// Waiting for another node to bootstrap.
    WaitingForBootstrap { deadline: Instant },
    /// Waiting for leader to add us to existing cluster.
    Joining { deadline: Instant },
    /// Cluster is active, monitoring for new peers.
    ClusterActive,
}

impl std::fmt::Display for CoordinatorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingForPeers => write!(f, "WAITING_FOR_PEERS"),
            Self::CheckingCluster => write!(f, "CHECKING_CLUSTER"),
            Self::CoordinatingBootstrap => write!(f, "COORDINATING_BOOTSTRAP"),
            Self::Bootstrapping => write!(f, "BOOTSTRAPPING"),
            Self::WaitingForBootstrap { .. } => write!(f, "WAITING_FOR_BOOTSTRAP"),
            Self::Joining { .. } => write!(f, "JOINING"),
            Self::ClusterActive => write!(f, "CLUSTER_ACTIVE"),
        }
    }
}

/// Coordinates cluster formation and membership.
///
/// # Responsibilities
///
/// 1. **Bootstrap coordination**: Determines which node should bootstrap
/// 2. **Cluster joining**: Waits for leader to add this node if cluster exists
/// 3. **New peer addition**: Monitors for new trusted peers (leader only)
///
/// # Usage
///
/// ```ignore
/// let coordinator = RaftCoordinator::new(node_id, raft_node.clone(), api_port);
/// tokio::spawn(async move { coordinator.run().await; });
/// ```
pub struct RaftCoordinator {
    /// This node's ID (derived from IP).
    node_id: NodeId,
    /// Reference to the RAFT node.
    raft_node: Arc<RaftNode>,
    /// HTTP client for peer health queries.
    http_client: reqwest::Client,
    /// Current state.
    state: CoordinatorState,
    /// Peers we've already added (to avoid duplicates).
    added_peers: HashSet<NodeId>,
}

impl RaftCoordinator {
    /// Create a new coordinator.
    pub fn new(node_id: NodeId, raft_node: Arc<RaftNode>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(HEALTH_QUERY_TIMEOUT)
            .connect_timeout(Duration::from_secs(3))
            .build()
            .expect("Failed to create HTTP client");

        Self { node_id, raft_node, http_client, state: CoordinatorState::WaitingForPeers, added_peers: HashSet::new() }
    }

    /// Run the coordinator state machine forever.
    pub async fn run(mut self) {
        info!(node_id = self.node_id, "RaftCoordinator started");

        loop {
            let next_state = match &self.state {
                CoordinatorState::WaitingForPeers => self.handle_waiting_for_peers().await,
                CoordinatorState::CheckingCluster => self.handle_checking_cluster().await,
                CoordinatorState::CoordinatingBootstrap => self.handle_coordinating_bootstrap().await,
                CoordinatorState::Bootstrapping => self.handle_bootstrapping().await,
                CoordinatorState::WaitingForBootstrap { deadline } => {
                    self.handle_waiting_for_bootstrap(*deadline).await
                }
                CoordinatorState::Joining { deadline } => self.handle_joining(*deadline).await,
                CoordinatorState::ClusterActive => self.handle_cluster_active().await,
            };

            if next_state != self.state {
                info!(from = %self.state, to = %next_state, "Coordinator state transition");
                self.state = next_state;
            }
        }
    }

    // ========================================================================
    // State Handlers
    // ========================================================================

    async fn handle_waiting_for_peers(&self) -> CoordinatorState {
        let trusted_count = TrustedPeerRegistry::get().trusted_count().await;

        if trusted_count >= MIN_PEERS_FOR_BOOTSTRAP {
            info!(trusted_count, "Sufficient peers, checking cluster state");
            return CoordinatorState::CheckingCluster;
        }

        debug!(trusted_count, min = MIN_PEERS_FOR_BOOTSTRAP, "Waiting for peers");
        tokio::time::sleep(DISCOVERY_INTERVAL).await;
        CoordinatorState::WaitingForPeers
    }

    async fn handle_checking_cluster(&self) -> CoordinatorState {
        let trusted_nodes = TrustedPeerRegistry::get().get_trusted_nodes().await;

        // Query all trusted peers for cluster state
        let mut found_leader: Option<(NodeId, SocketAddr)> = None;

        for peer_id in &trusted_nodes {
            if let Some(api_addr) = TrustedPeerRegistry::get().get_api_addr(*peer_id).await {
                if let Ok(health) = self.query_peer_health(api_addr).await {
                    if health.in_cluster() && health.is_leader() {
                        // Found the leader
                        if let Some(raft_addr) = TrustedPeerRegistry::get().get_raft_addr(*peer_id).await {
                            found_leader = Some((*peer_id, raft_addr));
                            break;
                        }
                    }
                }
            }
        }

        match found_leader {
            Some((leader_id, leader_addr)) => {
                info!(leader_id, %leader_addr, "Found existing cluster");
                self.raft_node.router().add_node(leader_id, leader_addr).await;
                CoordinatorState::Joining { deadline: Instant::now() + MEMBERSHIP_WAIT_TIMEOUT }
            }
            None => {
                info!("No existing cluster, coordinating bootstrap");
                CoordinatorState::CoordinatingBootstrap
            }
        }
    }

    async fn handle_coordinating_bootstrap(&self) -> CoordinatorState {
        let ready_set = self.build_mutual_ready_set().await;

        if ready_set.len() < BOOTSTRAP_QUORUM {
            debug!(ready_count = ready_set.len(), required = BOOTSTRAP_QUORUM, "Not enough mutually-ready nodes");
            tokio::time::sleep(DISCOVERY_INTERVAL).await;
            return CoordinatorState::CoordinatingBootstrap;
        }

        let lowest_ready = *ready_set.iter().min().unwrap();

        if lowest_ready == self.node_id {
            info!(ready_set_size = ready_set.len(), "This node will bootstrap");
            CoordinatorState::Bootstrapping
        } else {
            info!(lowest_ready, "Waiting for node {} to bootstrap", lowest_ready);
            CoordinatorState::WaitingForBootstrap { deadline: Instant::now() + BOOTSTRAP_WAIT_TIMEOUT }
        }
    }

    async fn handle_bootstrapping(&self) -> CoordinatorState {
        // Safety jitter to prevent race conditions
        let jitter_ms = 100 + (self.node_id % 500);
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

        // Re-verify no cluster exists and we're still lowest
        let ready_set = self.build_mutual_ready_set().await;
        if ready_set.len() < BOOTSTRAP_QUORUM {
            warn!("Ready set shrunk, aborting bootstrap");
            return CoordinatorState::CoordinatingBootstrap;
        }

        let lowest_ready = *ready_set.iter().min().unwrap();
        if lowest_ready != self.node_id {
            info!(new_lowest = lowest_ready, "No longer lowest, deferring");
            return CoordinatorState::WaitingForBootstrap { deadline: Instant::now() + BOOTSTRAP_WAIT_TIMEOUT };
        }

        // Get addresses for ready peers
        let peer_addresses = TrustedPeerRegistry::get().get_raft_addresses().await;
        let bootstrap_peers: std::collections::BTreeMap<NodeId, SocketAddr> =
            peer_addresses.into_iter().filter(|(id, _)| ready_set.contains(id)).collect();

        info!(cluster_size = bootstrap_peers.len() + 1, "Bootstrapping cluster");

        match self.raft_node.initialize_cluster(bootstrap_peers).await {
            Ok(()) => {
                info!("Cluster bootstrap successful");
                CoordinatorState::ClusterActive
            }
            Err(e) => {
                error!("Bootstrap failed: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                CoordinatorState::CoordinatingBootstrap
            }
        }
    }

    async fn handle_waiting_for_bootstrap(&self, deadline: Instant) -> CoordinatorState {
        if Instant::now() >= deadline {
            warn!("Timeout waiting for bootstrap, re-checking");
            return CoordinatorState::CoordinatingBootstrap;
        }

        // Check if cluster now exists
        let trusted_nodes = TrustedPeerRegistry::get().get_trusted_nodes().await;
        for peer_id in &trusted_nodes {
            if let Some(api_addr) = TrustedPeerRegistry::get().get_api_addr(*peer_id).await {
                if let Ok(health) = self.query_peer_health(api_addr).await {
                    if health.in_cluster() {
                        if let Some(raft_addr) = TrustedPeerRegistry::get().get_raft_addr(*peer_id).await {
                            info!(peer_id, "Cluster is now active");
                            self.raft_node.router().add_node(*peer_id, raft_addr).await;
                            return CoordinatorState::Joining { deadline: Instant::now() + MEMBERSHIP_WAIT_TIMEOUT };
                        }
                    }
                }
            }
        }

        // Check if we should take over as bootstrap node
        let ready_set = self.build_mutual_ready_set().await;
        if ready_set.len() >= BOOTSTRAP_QUORUM {
            let lowest_ready = *ready_set.iter().min().unwrap();
            if lowest_ready == self.node_id {
                info!("Taking over as bootstrap node");
                return CoordinatorState::Bootstrapping;
            }
        }

        tokio::time::sleep(DISCOVERY_INTERVAL).await;
        CoordinatorState::WaitingForBootstrap { deadline }
    }

    async fn handle_joining(&self, deadline: Instant) -> CoordinatorState {
        if Instant::now() >= deadline {
            error!("Timeout waiting to join cluster");
            return CoordinatorState::WaitingForPeers;
        }

        // Check our membership status
        if let Ok(state) = self.raft_node.get_node_state().await {
            if state.status.in_cluster() && !matches!(state.status, common::NodeStatus::Learner) {
                info!(status = %state.status, "Joined cluster");
                return CoordinatorState::ClusterActive;
            } else if matches!(state.status, common::NodeStatus::Learner) {
                debug!("Added as learner, waiting for promotion");
            } else {
                debug!(status = %state.status, "Waiting to be added");
            }
        }

        tokio::time::sleep(DISCOVERY_INTERVAL).await;
        CoordinatorState::Joining { deadline }
    }

    async fn handle_cluster_active(&mut self) -> CoordinatorState {
        // Only leader adds new peers
        if !self.raft_node.is_leader().await {
            tokio::time::sleep(PEER_MONITOR_INTERVAL).await;
            return CoordinatorState::ClusterActive;
        }

        // Get current cluster membership
        let metrics = self.raft_node.raft().metrics().borrow().clone();
        let current_members: HashSet<NodeId> = metrics
            .membership_config
            .membership()
            .voter_ids()
            .chain(metrics.membership_config.membership().learner_ids())
            .collect();

        // Get all trusted peers and their addresses
        let trusted = TrustedPeerRegistry::get().get_trusted_nodes().await;
        let addresses = TrustedPeerRegistry::get().get_raft_addresses().await;

        for peer_id in trusted {
            if current_members.contains(&peer_id) || self.added_peers.contains(&peer_id) {
                continue;
            }

            if let Some(&peer_addr) = addresses.get(&peer_id) {
                info!(peer_id, "Adding new trusted peer");

                match self.raft_node.add_peer(peer_id, peer_addr).await {
                    Ok(()) => {
                        self.added_peers.insert(peer_id);
                        info!(peer_id, "Peer added successfully");
                    }
                    Err(e) => {
                        warn!(peer_id, "Failed to add peer: {}", e);
                    }
                }
            }
        }

        tokio::time::sleep(PEER_MONITOR_INTERVAL).await;
        CoordinatorState::ClusterActive
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Build the set of mutually-ready peers for bootstrap.
    async fn build_mutual_ready_set(&self) -> HashSet<NodeId> {
        let mut ready_set = HashSet::new();

        // Check if we're ready
        let our_trusted = TrustedPeerRegistry::get().get_trusted_nodes().await;
        if our_trusted.len() >= MIN_PEERS_FOR_BOOTSTRAP {
            ready_set.insert(self.node_id);
        }

        // Check each trusted peer
        for peer_id in &our_trusted {
            if let Some(api_addr) = TrustedPeerRegistry::get().get_api_addr(*peer_id).await {
                if let Ok(health) = self.query_peer_health(api_addr).await {
                    if health.is_mutually_ready(self.node_id, MIN_PEERS_FOR_BOOTSTRAP) {
                        ready_set.insert(*peer_id);
                        debug!(peer_id, "Peer is mutually ready");
                    }
                }
            }
        }

        ready_set
    }

    /// Query a peer's health endpoint.
    async fn query_peer_health(&self, addr: SocketAddr) -> anyhow::Result<HealthResponse> {
        let url = format!("http://{}/health", addr);
        let response = self.http_client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Health query returned status {}", response.status());
        }

        let health: HealthResponse = response.json().await?;
        Ok(health)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinator_state_display() {
        assert_eq!(CoordinatorState::WaitingForPeers.to_string(), "WAITING_FOR_PEERS");
        assert_eq!(CoordinatorState::CheckingCluster.to_string(), "CHECKING_CLUSTER");
        assert_eq!(CoordinatorState::CoordinatingBootstrap.to_string(), "COORDINATING_BOOTSTRAP");
        assert_eq!(CoordinatorState::Bootstrapping.to_string(), "BOOTSTRAPPING");
        assert_eq!(
            CoordinatorState::WaitingForBootstrap { deadline: Instant::now() }.to_string(),
            "WAITING_FOR_BOOTSTRAP"
        );
        assert_eq!(CoordinatorState::Joining { deadline: Instant::now() }.to_string(), "JOINING");
        assert_eq!(CoordinatorState::ClusterActive.to_string(), "CLUSTER_ACTIVE");
    }

    #[test]
    fn test_health_response_helpers() {
        // Test the HealthResponse helper methods
        let bootstrapping = HealthResponse {
            node_id: 123,
            status: common::NodeStatus::Bootstrapping,
            term: 0,
            cluster_size: 0,
            trusted_peers: vec![456, 789],
        };

        assert!(bootstrapping.is_bootstrapping());
        assert!(!bootstrapping.in_cluster());
        assert!(bootstrapping.ready_for_bootstrap(2));
        assert!(bootstrapping.is_mutually_ready(456, 2));
        assert!(!bootstrapping.is_mutually_ready(999, 2));

        let leader = HealthResponse {
            node_id: 123,
            status: common::NodeStatus::Leader,
            term: 5,
            cluster_size: 3,
            trusted_peers: vec![456, 789],
        };

        assert!(leader.is_leader());
        assert!(leader.in_cluster());
        assert!(!leader.ready_for_bootstrap(2));
    }
}

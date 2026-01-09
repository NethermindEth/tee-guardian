// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Network layer for RAFT RPC with trust-gated communication.
//!
//! # Security Model
//!
//! The [`Router`] enforces that **all outbound RPC is trust-gated**:
//!
//! ```text
//! RaftNode                Router                 GuardianTrustedPeerSet
//!    │                      │                           │
//!    │── send_rpc(peer) ───▶│                           │
//!    │                      │── is_trusted(peer)? ─────▶│
//!    │                      │◀─ true/false ────────────│
//!    │                      │                           │
//!    │                      │ if false: return Unreachable
//!    │                      │ if true:  proceed with HTTP
//! ```
//!
//! OpenRaft handles `Unreachable` errors automatically - it will retry,
//! exclude the node from quorum calculations, and trigger leader election
//! if the leader becomes unreachable. This means the RAFT layer doesn't
//! need to know anything about attestation - it just sees network errors.
//!
//! # Security Properties
//!
//! - **No consensus with unattested peers**: Router blocks all RPC to untrusted nodes
//! - **No information leakage**: Heartbeats, votes, and logs never reach malicious nodes
//! - **Automatic recovery**: When a peer becomes trusted, Router automatically allows RPC
//!
//! # Connection Pooling
//!
//! The Router maintains a single `reqwest::Client` with connection pooling
//! (10 idle connections per host). This is important for RAFT performance
//! since heartbeats happen every 500ms.

use crate::types::{typ, NodeId, TypeConfig};
use bootstrap::TrustedPeerRegistry;
use openraft::error::{InstallSnapshotError, NetworkError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse, VoteRequest,
    VoteResponse,
};
use openraft::BasicNode;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Trust-gated RPC router for RAFT consensus.
///
/// # Trust Enforcement
///
/// All outbound RPC is blocked unless the target node passes the [`TrustedPeerRegistry`]
/// check. This is the **network-level security boundary** for the cluster.
///
/// The `TrustedPeerRegistry` is a global singleton populated by the bootstrap crate's
/// `PeerAttestationManager` after successful TDX attestation. The RAFT layer doesn't
/// need to know anything about attestation - it just sees `Unreachable` errors for
/// untrusted peers.
///
/// # Cloning
///
/// Clone is cheap - all fields are `Arc` or `Clone`. Cloned routers share:
/// - The same node address map
/// - The same HTTP connection pool
/// - Access to the same global `TrustedPeerRegistry` singleton
#[derive(Clone)]
pub struct Router {
    /// Node ID -> socket address mapping
    nodes: Arc<RwLock<BTreeMap<NodeId, SocketAddr>>>,
    /// Pooled HTTP client (10 idle connections per host, 10s timeout)
    http_client: reqwest::Client,
}

impl Router {
    /// Create a new trust-gated router.
    ///
    /// All outbound RPC will be blocked unless the target node is in the global
    /// `TrustedPeerRegistry` singleton. The registry must be initialized via
    /// `TrustedPeerRegistry::init()` before creating a Router.
    pub fn new() -> Self {
        // Verify the singleton is initialized (will panic with helpful message if not)
        let _ = TrustedPeerRegistry::get();

        Self {
            nodes: Arc::new(RwLock::new(BTreeMap::new())),
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5))
                .pool_max_idle_per_host(10)
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    pub async fn add_node(&self, node_id: NodeId, address: SocketAddr) {
        self.nodes.write().await.insert(node_id, address);
    }

    pub async fn remove_node(&self, node_id: NodeId) {
        self.nodes.write().await.remove(&node_id);
    }

    /// Send RPC to a target node with trust checking.
    ///
    /// # Security
    ///
    /// Blocks RPC to untrusted peers by returning `Unreachable` error.
    /// OpenRaft treats this as a temporary failure and will retry, which
    /// continues to fail until the peer is attested and added to the registry.
    async fn send_rpc<Req, Resp, Err>(
        &self,
        target: NodeId,
        target_node: &BasicNode,
        uri: &str,
        req: Req,
    ) -> Result<Resp, openraft::error::RPCError<NodeId, BasicNode, Err>>
    where
        Req: Serialize,
        Err: std::error::Error + DeserializeOwned,
        Resp: DeserializeOwned,
    {
        // SECURITY: Block RPC to unattested peers using global singleton
        if !TrustedPeerRegistry::get().is_trusted(target).await {
            debug!("Blocking RPC to untrusted peer: {}", target);
            return Err(openraft::error::RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Peer not trusted",
            ))));
        }

        let url = format!("http://{}/{}", target_node.addr, uri);
        debug!("send_rpc to url: {}", url);

        let resp = self.http_client.post(&url).json(&req).send().await.map_err(|e| {
            if e.is_connect() {
                return openraft::error::RPCError::Unreachable(Unreachable::new(&e));
            }
            openraft::error::RPCError::Network(NetworkError::new(&e))
        })?;

        // Check HTTP status code
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(
                target = target,
                uri = uri,
                status = %status,
                body = %body,
                "RPC request failed with non-2xx status"
            );
            return Err(openraft::error::RPCError::Network(NetworkError::new(&std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("HTTP {}: {}", status, body),
            ))));
        }

        // Deserialize response directly (not wrapped in Result)
        // The HTTP handlers return responses directly, not Result<Response, Error>
        let res: Resp = resp.json().await.map_err(|e| {
            tracing::error!(
                target = target,
                uri = uri,
                error = %e,
                "Failed to deserialize RPC response"
            );
            openraft::error::RPCError::Network(NetworkError::new(&e))
        })?;

        Ok(res)
    }
}

impl RaftNetworkFactory<TypeConfig> for Router {
    type Network = Connection;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        Connection { owner: self.clone(), target, target_node: node.clone() }
    }
}

/// A connection to a specific peer for OpenRaft RPC.
///
/// Created by [`Router::new_client`] for each peer. The connection holds
/// a clone of the Router (cheap, shares underlying state) and target info.
pub struct Connection {
    owner: Router,
    target: NodeId,
    target_node: BasicNode,
}

impl RaftNetwork<TypeConfig> for Connection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, typ::RPCError> {
        self.owner.send_rpc(self.target, &self.target_node, "raft-append", req).await
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<NodeId>, typ::RPCError<InstallSnapshotError>> {
        self.owner.send_rpc(self.target, &self.target_node, "raft-snapshot", req).await
    }

    async fn vote(
        &mut self,
        req: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, typ::RPCError> {
        self.owner.send_rpc(self.target, &self.target_node, "raft-vote", req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Helper to get a clean registry and router for testing
    async fn setup_test_router() -> Router {
        let registry = TrustedPeerRegistry::init_or_get();
        registry.clear().await;
        Router::new()
    }

    #[tokio::test]
    #[serial]
    async fn test_router_new() {
        let router = setup_test_router().await;
        // Router should be empty initially
        assert_eq!(router.nodes.read().await.len(), 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_router_clone() {
        let router = setup_test_router().await;
        let cloned = router.clone();
        // Both should share the same underlying Arc
        assert_eq!(Arc::as_ptr(&router.nodes), Arc::as_ptr(&cloned.nodes));
    }

    #[tokio::test]
    #[serial]
    async fn test_add_node() {
        let router = setup_test_router().await;
        let addr: SocketAddr = "192.168.1.1:8444".parse().unwrap();

        router.add_node(1, addr).await;
        let nodes = router.nodes.read().await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes.get(&1), Some(&addr));
    }

    #[tokio::test]
    #[serial]
    async fn test_add_multiple_nodes() {
        let router = setup_test_router().await;
        let addr1: SocketAddr = "192.168.1.1:8444".parse().unwrap();
        let addr2: SocketAddr = "192.168.1.2:8444".parse().unwrap();
        let addr3: SocketAddr = "192.168.1.3:8444".parse().unwrap();

        router.add_node(1, addr1).await;
        router.add_node(2, addr2).await;
        router.add_node(3, addr3).await;

        let nodes = router.nodes.read().await;
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes.get(&1), Some(&addr1));
        assert_eq!(nodes.get(&2), Some(&addr2));
        assert_eq!(nodes.get(&3), Some(&addr3));
    }

    #[tokio::test]
    #[serial]
    async fn test_add_node_updates_existing() {
        let router = setup_test_router().await;
        let addr1: SocketAddr = "192.168.1.1:8444".parse().unwrap();
        let addr2: SocketAddr = "192.168.1.2:9999".parse().unwrap();

        router.add_node(1, addr1).await;
        router.add_node(1, addr2).await; // Update same node

        let nodes = router.nodes.read().await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes.get(&1), Some(&addr2)); // Should have new address
    }

    #[tokio::test]
    #[serial]
    async fn test_remove_node() {
        let router = setup_test_router().await;
        let addr: SocketAddr = "192.168.1.1:8444".parse().unwrap();

        router.add_node(1, addr).await;
        router.remove_node(1).await;

        let nodes = router.nodes.read().await;
        assert_eq!(nodes.len(), 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_remove_nonexistent_node() {
        let router = setup_test_router().await;

        // Removing non-existent node should not panic
        router.remove_node(999).await;

        let nodes = router.nodes.read().await;
        assert_eq!(nodes.len(), 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_concurrent_access() {
        let router = setup_test_router().await;
        let router1 = router.clone();
        let router2 = router.clone();

        let addr1: SocketAddr = "192.168.1.1:8444".parse().unwrap();
        let addr2: SocketAddr = "192.168.1.2:8444".parse().unwrap();

        // Spawn concurrent tasks
        let h1 = tokio::spawn(async move {
            router1.add_node(1, addr1).await;
        });

        let h2 = tokio::spawn(async move {
            router2.add_node(2, addr2).await;
        });

        h1.await.unwrap();
        h2.await.unwrap();

        let nodes = router.nodes.read().await;
        assert_eq!(nodes.len(), 2);
    }

    #[tokio::test]
    #[serial]
    async fn test_trust_check_untrusted_peer() {
        let _ = setup_test_router().await;
        let registry = TrustedPeerRegistry::get();

        // Node 999 is NOT trusted
        assert!(!registry.is_trusted(999).await);
    }

    fn test_addr(node: u8) -> std::net::SocketAddr {
        format!("192.168.1.{}:8444", node).parse().unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_trust_check_trusted_peer() {
        let _ = setup_test_router().await;
        let registry = TrustedPeerRegistry::get();

        // Mark node 123 as trusted
        registry.mark_trusted(123, test_addr(123), test_addr(123)).await;

        // Verify trust check passes
        assert!(registry.is_trusted(123).await);
    }

    #[tokio::test]
    #[serial]
    async fn test_trust_revocation() {
        let _ = setup_test_router().await;
        let registry = TrustedPeerRegistry::get();

        // Initially trust node 123
        registry.mark_trusted(123, test_addr(123), test_addr(123)).await;
        assert!(registry.is_trusted(123).await);

        // Revoke trust
        registry.revoke_trust(123).await;
        assert!(!registry.is_trusted(123).await);
    }

    #[tokio::test]
    #[serial]
    async fn test_router_uses_global_registry() {
        let router = setup_test_router().await;
        let registry = TrustedPeerRegistry::get();

        // Add peer via registry
        registry.mark_trusted(456, test_addr(100), test_addr(100)).await;

        // Router should see the trusted peer (uses same global singleton)
        assert!(registry.is_trusted(456).await);

        // Create another router - should see the same state
        let router2 = Router::new();
        // Both routers use the same global registry
        assert!(TrustedPeerRegistry::get().is_trusted(456).await);

        // Verify both routers have independent node maps but shared trust registry
        router.add_node(1, "192.168.1.1:8444".parse().unwrap()).await;
        assert_eq!(router.nodes.read().await.len(), 1);
        assert_eq!(router2.nodes.read().await.len(), 0); // Independent node maps
    }
}

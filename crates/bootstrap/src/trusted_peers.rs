// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Trusted peer registry - the authoritative source for guardian cluster membership.
//!
//! # Security Architecture
//!
//! The [`TrustedPeerRegistry`] is the **sole authority** for cluster membership.
//! A node can only participate in RAFT consensus if its `NodeId` is in this registry.
//!
//! ```text
//!                    ┌─────────────────────┐
//!                    │  TrustedPeerRegistry │
//!                    │    (global singleton)│
//!                    └──────────┬──────────┘
//!                               │
//!            ┌──────────────────┼──────────────────┐
//!            │                  │                  │
//!            ▼                  ▼                  ▼
//!     ┌────────────┐    ┌────────────┐    ┌────────────┐
//!     │   Router   │    │  RaftNode  │    │Coordinator │
//!     │ (RPC gate) │    │ (cluster)  │    │ (bootstrap)│
//!     └────────────┘    └────────────┘    └────────────┘
//! ```
//!
//! # Singleton Pattern
//!
//! This module enforces that exactly ONE `TrustedPeerRegistry` exists per process.
//! Use [`TrustedPeerRegistry::init()`] at startup, then [`TrustedPeerRegistry::get()`]
//! everywhere else.
//!
//! # Thread Safety
//!
//! The registry uses `RwLock<BTreeMap>` for concurrent access from multiple async tasks.
//! All methods are `async` and acquire appropriate locks.
//!
//! # Persistence
//!
//! The registry is **not persisted** - it's rebuilt on each boot via mutual attestation
//! during cluster formation. This is intentional: trust is based on live attestation,
//! not cached state that could become stale.

use peer_registry::types::NodeId;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::info;

/// Global singleton instance.
static INSTANCE: OnceLock<TrustedPeerRegistry> = OnceLock::new();

/// Information about a trusted peer.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// RAFT RPC address (IP:port).
    pub raft_addr: SocketAddr,
    /// Guardian API address (IP:port).
    pub api_addr: SocketAddr,
}

/// Global registry of attested peer node IDs - the **single source of truth** for
/// guardian cluster membership authorization.
///
/// # Singleton Pattern
///
/// This struct is a **global singleton**. There is exactly ONE instance per process.
///
/// ```rust,ignore
/// use bootstrap::TrustedPeerRegistry;
///
/// // At startup (once):
/// TrustedPeerRegistry::init();
///
/// // Everywhere else:
/// let registry = TrustedPeerRegistry::get();
/// registry.mark_trusted(node_id, raft_addr, api_addr).await;
/// ```
///
/// # Security Model
///
/// This registry gates all inter-node communication in the guardian cluster:
///
/// - **Router**: Blocks RPC to any node not in this registry
/// - **RaftNode**: Rejects join requests from untrusted nodes  
/// - **RaftCoordinator**: Uses this for cluster formation decisions
///
/// A node is added to this registry **only** after successful TDX attestation
/// verification. Trust can be revoked at any time, immediately blocking all
/// communication with that node.
pub struct TrustedPeerRegistry {
    /// Map of node_id -> peer info (addresses).
    trusted_nodes: RwLock<BTreeMap<NodeId, PeerInfo>>,
}

impl TrustedPeerRegistry {
    /// Initialize the global singleton registry.
    ///
    /// # Panics
    ///
    /// Panics if called more than once. This is intentional - creating multiple
    /// registries would be a critical security bug.
    pub fn init() {
        let created = INSTANCE.set(Self { trusted_nodes: RwLock::new(BTreeMap::new()) });

        if created.is_err() {
            panic!(
                "BUG: TrustedPeerRegistry::init() called more than once!\n\
                 The registry must be initialized exactly once at startup.\n\
                 Use TrustedPeerRegistry::get() to access the singleton."
            );
        }

        info!("TrustedPeerRegistry initialized");
    }

    /// Get the global singleton registry.
    ///
    /// # Panics
    ///
    /// Panics if `init()` was not called first.
    pub fn get() -> &'static Self {
        INSTANCE.get().expect(
            "BUG: TrustedPeerRegistry::get() called before init()!\n\
             Call TrustedPeerRegistry::init() at startup before accessing the registry.",
        )
    }

    /// Initialize if not already initialized, then return reference.
    ///
    /// This is useful for tests where multiple tests may try to initialize
    /// the singleton.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn init_or_get() -> &'static Self {
        INSTANCE.get_or_init(|| Self { trusted_nodes: RwLock::new(BTreeMap::new()) })
    }

    /// Mark a node as trusted with its addresses.
    ///
    /// Called after successful TDX attestation verification.
    /// Idempotent - calling multiple times for same node_id updates the addresses.
    pub async fn mark_trusted(&self, node_id: NodeId, raft_addr: SocketAddr, api_addr: SocketAddr) {
        let mut trusted = self.trusted_nodes.write().await;
        let is_new = !trusted.contains_key(&node_id);
        trusted.insert(node_id, PeerInfo { raft_addr, api_addr });

        if is_new {
            info!(node_id = node_id, %raft_addr, %api_addr, "Peer marked as trusted");
        }
    }

    /// Check if a node is trusted.
    pub async fn is_trusted(&self, node_id: NodeId) -> bool {
        self.trusted_nodes.read().await.contains_key(&node_id)
    }

    /// Get the count of trusted peers.
    pub async fn trusted_count(&self) -> usize {
        self.trusted_nodes.read().await.len()
    }

    /// Get all trusted node IDs.
    pub async fn get_trusted_nodes(&self) -> Vec<NodeId> {
        self.trusted_nodes.read().await.keys().copied().collect()
    }

    /// Get RAFT addresses for all trusted peers.
    ///
    /// Used by RaftCoordinator for cluster formation.
    pub async fn get_raft_addresses(&self) -> BTreeMap<NodeId, SocketAddr> {
        self.trusted_nodes.read().await.iter().map(|(&id, info)| (id, info.raft_addr)).collect()
    }

    /// Get API address for a specific peer.
    ///
    /// Used for health endpoint queries during bootstrap coordination.
    pub async fn get_api_addr(&self, node_id: NodeId) -> Option<SocketAddr> {
        self.trusted_nodes.read().await.get(&node_id).map(|info| info.api_addr)
    }

    /// Get RAFT address for a specific peer.
    pub async fn get_raft_addr(&self, node_id: NodeId) -> Option<SocketAddr> {
        self.trusted_nodes.read().await.get(&node_id).map(|info| info.raft_addr)
    }

    /// Revoke trust from a node.
    ///
    /// Called when a peer's measurement is revoked or on security events.
    pub async fn revoke_trust(&self, node_id: NodeId) {
        let mut trusted = self.trusted_nodes.write().await;
        if trusted.remove(&node_id).is_some() {
            info!(node_id = node_id, "Peer trust revoked");
        }
    }

    /// Clear all trusted peers.
    ///
    /// Typically called during shutdown or cluster reset.
    pub async fn clear(&self) {
        let mut trusted = self.trusted_nodes.write().await;
        let count = trusted.len();
        trusted.clear();
        info!(count = count, "Cleared all trusted peers");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), port)
    }

    /// Helper to get a clean singleton for testing.
    async fn get_clean_registry() -> &'static TrustedPeerRegistry {
        let registry = TrustedPeerRegistry::init_or_get();
        registry.clear().await;
        registry
    }

    #[tokio::test]
    #[serial]
    async fn test_singleton_init_or_get() {
        let registry = get_clean_registry().await;
        assert_eq!(registry.trusted_count().await, 0);

        let registry2 = TrustedPeerRegistry::init_or_get();
        assert_eq!(registry2.trusted_count().await, 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_mark_trusted() {
        let registry = get_clean_registry().await;
        registry.mark_trusted(1, test_addr(8444), test_addr(8443)).await;
        assert!(registry.is_trusted(1).await);
        assert_eq!(registry.trusted_count().await, 1);
    }

    #[tokio::test]
    #[serial]
    async fn test_is_not_trusted() {
        let registry = get_clean_registry().await;
        assert!(!registry.is_trusted(999).await);
    }

    #[tokio::test]
    #[serial]
    async fn test_revoke_trust() {
        let registry = get_clean_registry().await;
        registry.mark_trusted(1, test_addr(8444), test_addr(8443)).await;
        assert!(registry.is_trusted(1).await);

        registry.revoke_trust(1).await;
        assert!(!registry.is_trusted(1).await);
    }

    #[tokio::test]
    #[serial]
    async fn test_get_trusted_nodes() {
        let registry = get_clean_registry().await;
        registry.mark_trusted(1, test_addr(8444), test_addr(8443)).await;
        registry.mark_trusted(2, test_addr(8444), test_addr(8443)).await;
        registry.mark_trusted(3, test_addr(8444), test_addr(8443)).await;

        let trusted = registry.get_trusted_nodes().await;
        assert_eq!(trusted.len(), 3);
        assert!(trusted.contains(&1));
        assert!(trusted.contains(&2));
        assert!(trusted.contains(&3));
    }

    #[tokio::test]
    #[serial]
    async fn test_get_addresses() {
        let registry = get_clean_registry().await;
        let raft_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8444);
        let api_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8443);

        registry.mark_trusted(1, raft_addr, api_addr).await;

        assert_eq!(registry.get_raft_addr(1).await, Some(raft_addr));
        assert_eq!(registry.get_api_addr(1).await, Some(api_addr));

        let raft_addrs = registry.get_raft_addresses().await;
        assert_eq!(raft_addrs.len(), 1);
        assert_eq!(raft_addrs.get(&1), Some(&raft_addr));
    }

    #[tokio::test]
    #[serial]
    async fn test_update_addresses() {
        let registry = get_clean_registry().await;
        let addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8444);
        let addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101)), 8444);

        registry.mark_trusted(1, addr1, addr1).await;
        assert_eq!(registry.get_raft_addr(1).await, Some(addr1));

        // Update with new address
        registry.mark_trusted(1, addr2, addr2).await;
        assert_eq!(registry.get_raft_addr(1).await, Some(addr2));

        // Still only one entry
        assert_eq!(registry.trusted_count().await, 1);
    }

    #[tokio::test]
    #[serial]
    async fn test_clear_registry() {
        let registry = get_clean_registry().await;
        registry.mark_trusted(1, test_addr(8444), test_addr(8443)).await;
        registry.mark_trusted(2, test_addr(8444), test_addr(8443)).await;
        assert_eq!(registry.trusted_count().await, 2);

        registry.clear().await;
        assert_eq!(registry.trusted_count().await, 0);
        assert!(!registry.is_trusted(1).await);
    }

    #[tokio::test]
    #[serial]
    async fn test_concurrent_modifications() {
        let registry = get_clean_registry().await;

        let mut handles = vec![];
        for i in 0..10 {
            handles.push(tokio::spawn(async move {
                TrustedPeerRegistry::get()
                    .mark_trusted(
                        i,
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, i as u8)), 8444),
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, i as u8)), 8443),
                    )
                    .await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(registry.trusted_count().await, 10);
    }
}

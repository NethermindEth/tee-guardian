// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Gossiped peer cache.
//!
//! Thread-safe cache for peer advertisements received via gossip.

use super::types::PeerAdvertisement;
use peer_registry::types::{BootstrapNode, NodeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::debug;

/// Cache for peer advertisements received via gossip.
///
/// All advertisements in this cache have been TEE-verified (signature + binding).
/// The cache is merged with registry peers during attestation discovery.
#[derive(Clone)]
pub struct GossipedPeerCache {
    /// Cached peer advertisements (node_id -> (advertisement, received_at))
    peers: Arc<RwLock<HashMap<NodeId, (PeerAdvertisement, Instant)>>>,
}

impl GossipedPeerCache {
    /// Create new empty cache.
    pub fn new() -> Self {
        Self { peers: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Add a verified peer advertisement.
    ///
    /// Only updates if the new advertisement is newer than existing one.
    /// Caller must verify the advertisement before calling this.
    pub async fn add_verified(&self, ad: PeerAdvertisement) {
        let node_id = ad.node_id;
        let mut peers = self.peers.write().await;

        // Check if this is newer than existing advertisement
        if let Some((existing, _)) = peers.get(&node_id) {
            if existing.timestamp >= ad.timestamp {
                debug!(node_id = node_id, "Ignoring older/duplicate advertisement");
                return;
            }
        }

        peers.insert(node_id, (ad, Instant::now()));
        debug!(node_id = node_id, "Added verified peer to gossip cache");
    }

    /// Check if a node_id is already in the cache.
    pub async fn contains(&self, node_id: NodeId) -> bool {
        let peers = self.peers.read().await;
        peers.contains_key(&node_id)
    }

    /// Get all cached peers as `BootstrapNode` format (for `PeerAttestationManager`).
    pub async fn get_all(&self) -> Vec<BootstrapNode> {
        let peers = self.peers.read().await;
        peers
            .values()
            .filter_map(|(ad, _)| {
                // Extract IP from raft_address
                match ad.raft_address.ip() {
                    std::net::IpAddr::V4(ipv4) => Some(BootstrapNode {
                        ip_address: ipv4,
                        raft_port: ad.raft_address.port(),
                        api_port: ad.api_address.port(),
                    }),
                    std::net::IpAddr::V6(ipv6) => {
                        panic!(
                            "BUG: IPv6 address {} in gossip cache for node {} - system is IPv4-only",
                            ipv6, ad.node_id
                        );
                    }
                }
            })
            .collect()
    }

    /// Get specific peer advertisement.
    pub async fn get(&self, node_id: NodeId) -> Option<PeerAdvertisement> {
        let peers = self.peers.read().await;
        peers.get(&node_id).map(|(ad, _)| ad.clone())
    }

    /// Get all advertisements (for API endpoint).
    pub async fn get_all_advertisements(&self) -> Vec<PeerAdvertisement> {
        let peers = self.peers.read().await;
        peers.values().map(|(ad, _)| ad.clone()).collect()
    }
}

impl Default for GossipedPeerCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gossip_cache_add_and_get() {
        let cache = GossipedPeerCache::new();

        let ad = PeerAdvertisement::new(
            123456,
            "192.168.1.20:8444".parse().unwrap(),
            "192.168.1.20:8443".parse().unwrap(),
            "a".repeat(64),
            "b".repeat(64),
            String::new(),
        );

        cache.add_verified(ad.clone()).await;

        let result = cache.get(123456).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().node_id, 123456);
    }

    #[tokio::test]
    async fn test_gossip_cache_newer_replaces_older() {
        let cache = GossipedPeerCache::new();

        // Create older advertisement
        let mut ad_old = PeerAdvertisement::new(
            123456,
            "192.168.1.20:8444".parse().unwrap(),
            "192.168.1.20:8443".parse().unwrap(),
            "a".repeat(64),
            "b".repeat(64),
            String::new(),
        );
        ad_old.timestamp = 1000;

        cache.add_verified(ad_old).await;

        // Create newer advertisement
        let mut ad_new = PeerAdvertisement::new(
            123456,
            "192.168.1.21:8444".parse().unwrap(),
            "192.168.1.21:8443".parse().unwrap(),
            "c".repeat(64),
            "d".repeat(64),
            String::new(),
        );
        ad_new.timestamp = 2000;

        cache.add_verified(ad_new).await;

        let result = cache.get(123456).await.unwrap();
        assert_eq!(result.measurement_hash, "c".repeat(64));
    }

    #[tokio::test]
    async fn test_gossip_cache_contains() {
        let cache = GossipedPeerCache::new();

        assert!(!cache.contains(123456).await);

        let ad = PeerAdvertisement::new(
            123456,
            "192.168.1.20:8444".parse().unwrap(),
            "192.168.1.20:8443".parse().unwrap(),
            "a".repeat(64),
            "b".repeat(64),
            String::new(),
        );

        cache.add_verified(ad).await;

        assert!(cache.contains(123456).await);
    }
}

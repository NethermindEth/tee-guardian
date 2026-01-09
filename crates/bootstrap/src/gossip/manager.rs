// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Peer gossip manager.
//!
//! Implements hybrid push-pull gossip with TEE-backed authentication.

use super::cache::GossipedPeerCache;
use super::config::GossipConfig;
use super::evidence::generate_gossip_evidence;
use super::types::{PeerAdvertisement, PeerListResponse};
use crate::trusted_peers::TrustedPeerRegistry;
use peer_registry::measurement_hash_from_evidence;
use peer_registry::types::{node_id_to_ip, NodeId};
use peer_registry::MeasurementRegistryClient;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Peer Gossip Manager.
///
/// Implements hybrid push-pull gossip with TEE-backed authentication:
/// - Pull: Periodically request peer lists from random trusted peers
/// - Push: Broadcast self-advertisement when cluster membership changes
/// - Initial: Advertise to all registry nodes on startup
///
/// All advertisements include TEE evidence binding addresses to the instance.
pub struct PeerGossipManager {
    node_id: NodeId,
    raft_address: SocketAddr,
    api_address: SocketAddr,
    instance_id: String,
    gossip_cache: Arc<GossipedPeerCache>,
    config: GossipConfig,
    registry_client: MeasurementRegistryClient,
    namespace: String,
    /// HTTP client for gossip requests (connection pooling)
    http_client: reqwest::Client,
    /// Cached evidence (generated lazily on first advertisement)
    cached_evidence: RwLock<Option<CachedEvidence>>,
}

/// Cached TEE evidence for advertisements.
struct CachedEvidence {
    evidence: String,
    measurement_hash: String,
}

impl PeerGossipManager {
    /// Create new gossip manager.
    ///
    /// # Panics
    ///
    /// Panics if `TrustedPeerRegistry::init()` was not called first.
    pub fn new(
        node_id: NodeId,
        raft_address: SocketAddr,
        api_address: SocketAddr,
        instance_id: String,
        registry_url: String,
        namespace: String,
    ) -> Self {
        // Verify singleton is initialized
        let _ = TrustedPeerRegistry::get();

        let gossip_cache = Arc::new(GossipedPeerCache::new());
        let config = GossipConfig::default();

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(10)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            node_id,
            raft_address,
            api_address,
            instance_id,
            gossip_cache,
            config,
            registry_client: MeasurementRegistryClient::new(registry_url),
            namespace,
            http_client,
            cached_evidence: RwLock::new(None),
        }
    }

    /// Generate and cache TEE evidence for advertisements.
    ///
    /// This is called lazily on first advertisement to avoid blocking startup.
    async fn get_or_generate_evidence(&self) -> Result<(String, String), String> {
        // Check cache first
        {
            let cache = self.cached_evidence.read().await;
            if let Some(cached) = &*cache {
                return Ok((cached.evidence.clone(), cached.measurement_hash.clone()));
            }
        }

        // Generate evidence
        info!("Generating TEE evidence for gossip advertisements");
        let evidence = generate_gossip_evidence(&self.raft_address, &self.api_address, &self.instance_id).await?;

        // Compute measurement hash from evidence
        let measurement_hash = measurement_hash_from_evidence(&evidence).map_err(|e| e.to_string())?;

        // Cache it
        {
            let mut cache = self.cached_evidence.write().await;
            *cache = Some(CachedEvidence { evidence: evidence.clone(), measurement_hash: measurement_hash.clone() });
        }

        Ok((evidence, measurement_hash))
    }

    /// Advertise to all nodes from the measurement registry.
    ///
    /// Called on startup to announce this node's presence to the cluster.
    /// This triggers wake-up signals on receiving nodes, enabling fast mutual attestation.
    pub async fn initial_advertise(&self) {
        info!("Sending initial advertisement to all registry nodes");

        // Get all nodes from registry - this is a startup invariant
        let registry_nodes = self
            .registry_client
            .get_nodes(&self.namespace)
            .await
            .expect("Failed to query measurement registry - cannot join cluster");

        assert!(!registry_nodes.is_empty(), "No nodes in registry - cannot join cluster");

        // Create self-advertisement
        let ad = match self.create_advertisement().await {
            Ok(ad) => ad,
            Err(e) => {
                warn!("Failed to create advertisement: {}", e);
                return;
            }
        };

        info!(
            node_count = registry_nodes.len(),
            has_evidence = !ad.evidence.is_empty(),
            "Broadcasting initial advertisement to registry nodes"
        );

        // Send to all registry nodes (except self)
        for node in registry_nodes {
            let peer_node_id = peer_registry::types::ip_to_node_id(&node.ip_address);
            if peer_node_id == self.node_id {
                continue;
            }

            self.send_advertisement_to_addr(node.ip_address, node.api_port, ad.clone()).await;
        }
    }

    /// Run gossip manager forever (pull loop).
    pub async fn run(self) -> ! {
        info!("Peer gossip manager started (pull every {}s)", self.config.pull_interval_secs);

        loop {
            // Pull peer list from random trusted peer
            self.pull_peer_list().await;

            tokio::time::sleep(Duration::from_secs(self.config.pull_interval_secs)).await;
        }
    }

    /// Pull peer list from a random trusted peer.
    async fn pull_peer_list(&self) {
        let trusted_nodes = TrustedPeerRegistry::get().get_trusted_nodes().await;

        if trusted_nodes.is_empty() {
            debug!("No trusted peers to pull from");
            return;
        }

        // Pick random trusted peer (ensure rng is dropped before any await)
        let random_peer = {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            let Some(&peer) = trusted_nodes.choose(&mut rng) else {
                return;
            };
            peer
        };

        // Skip self
        if random_peer == self.node_id {
            return;
        }

        debug!(peer = random_peer, "Pulling peer list from trusted peer");

        let peer_ip = node_id_to_ip(random_peer);

        // Get API port for this peer
        let api_port = TrustedPeerRegistry::get()
            .get_api_addr(random_peer)
            .await
            .expect("BUG: trusted peer must have api_addr")
            .port();

        // Request peer list via Guardian Node API
        let url = format!("http://{}:{}/guardian_node/peers", peer_ip, api_port);

        match self.http_client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<PeerListResponse>().await {
                        Ok(peer_list) => {
                            info!(peer_count = peer_list.peers.len(), "Received peer advertisements");

                            // Validate and add to cache
                            for ad in peer_list.peers {
                                if let Err(e) = self.validate_and_add_advertisement(ad).await {
                                    debug!("Rejected advertisement: {}", e);
                                }
                            }
                        }
                        Err(e) => warn!("Failed to parse peer list: {}", e),
                    }
                }
            }
            Err(e) => debug!("Failed to pull peer list: {}", e),
        }
    }

    /// Validate advertisement and add to cache.
    ///
    /// Performs:
    /// 1. Format validation
    /// 2. Age check
    /// 3. TEE evidence verification (signature + address binding)
    pub async fn validate_and_add_advertisement(&self, ad: PeerAdvertisement) -> Result<(), String> {
        // 1. Validate format
        ad.validate_format()?;

        // 2. Check timestamp not too old
        let now = common::unix_timestamp_secs();
        let age_hours = (now.saturating_sub(ad.timestamp)) / 3600;

        if age_hours > self.config.max_advertisement_age_hours {
            return Err(format!("Advertisement too old: {} hours", age_hours));
        }

        // 3. Skip self
        if ad.node_id == self.node_id {
            return Ok(());
        }

        // 4. Skip if already in cache (avoid re-verification overhead)
        if self.gossip_cache.contains(ad.node_id).await {
            // Check if incoming is newer
            if let Some(existing) = self.gossip_cache.get(ad.node_id).await {
                if existing.timestamp >= ad.timestamp {
                    return Ok(()); // Already have this or newer
                }
            }
        }

        // 5. Verify TEE evidence (signature + address binding)
        ad.verify_evidence().await?;

        // 6. Add to cache
        self.gossip_cache.add_verified(ad).await;

        Ok(())
    }

    /// Create a self-advertisement with TEE evidence.
    async fn create_advertisement(&self) -> Result<PeerAdvertisement, String> {
        let (evidence, measurement_hash) = self.get_or_generate_evidence().await?;

        Ok(PeerAdvertisement::new(
            self.node_id,
            self.raft_address,
            self.api_address,
            measurement_hash,
            self.instance_id.clone(),
            evidence,
        ))
    }

    /// Send advertisement to a specific address.
    async fn send_advertisement_to_addr(&self, ip: std::net::Ipv4Addr, port: u16, ad: PeerAdvertisement) {
        let url = format!("http://{}:{}/guardian_node/advertise", ip, port);

        match self.http_client.post(&url).json(&ad).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    debug!(peer_ip = %ip, "Advertisement sent successfully");
                } else {
                    debug!(peer_ip = %ip, status = %response.status(), "Peer rejected advertisement");
                }
            }
            Err(e) => debug!(peer_ip = %ip, "Failed to send advertisement: {}", e),
        }
    }
}

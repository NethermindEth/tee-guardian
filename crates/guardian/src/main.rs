// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Guardian TEE-KMS service entrypoint.

use anyhow::Result;
use peer_registry::{detect_available_tee, get_measurement_hash, InstanceId};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Get node IP from DHCP
    let public_ip = common::get_node_ip_from_dhcp()?;

    if let Err(e) = common::logging::init_logging(public_ip) {
        eprintln!("Failed to initialize logging: {e}");
    }

    info!("Starting Guardian TEE-KMS Service");

    // Detect TEE type
    let tee_type = detect_available_tee().ok_or_else(|| anyhow::anyhow!("No TEE available"))?;

    let node_id = bootstrap::ip_to_node_id(&public_ip);
    let instance_id = InstanceId::load_for_tee(tee_type, common::config::INSTANCE_ID_PATH).await?;
    info!("Node ID: {} (IP: {}) [Instance ID: {}]", node_id, public_ip, instance_id);

    let measurement_hash = get_measurement_hash(tee_type).await?;
    info!("Measurement hash: {}...", &measurement_hash[..16]);

    // Initialize global state
    bootstrap::TrustedPeerRegistry::init();

    // Configuration
    let registry_url = common::config::MEASUREMENT_REGISTRY_URL;

    // Attestation API with event handling
    let attestation_state = bootstrap::AttestationApiState::new(registry_url.to_string());
    tokio::spawn({
        let handler = bootstrap::AttestationEventHandler::new(
            common::config::GUARDIAN_CERTIFICATE_NAMESPACE.to_string(),
            attestation_state.subscribe(),
        );
        async move { handler.run().await }
    });
    tokio::spawn(bootstrap::cleanup_task(attestation_state.peer_registry.clone()));

    // Gossip cache (shared between API and gossip manager)
    let gossip_cache = Arc::new(bootstrap::GossipedPeerCache::new());

    // API state
    let api_state = guardian::ApiState::new(node_id);
    let guardian_node_state = bootstrap::GuardianNodeApiState::new(
        node_id,
        attestation_state,
        gossip_cache.clone(),
        bootstrap::GossipConfig::default(),
    );

    // Start RAFT API server
    let raft_api_state = consensus::raft_api_server::RaftApiState::new();

    tokio::spawn({
        let state = raft_api_state.clone();
        async move {
            if let Err(e) = consensus::start_raft_api_server(state).await {
                tracing::error!("RAFT API server failed: {}", e);
            }
        }
    });

    // Start combined Guardian API server
    tokio::spawn({
        let api_state = api_state.clone();
        let guardian_node_state = guardian_node_state.clone();

        async move {
            if let Err(e) = guardian::start_guardian_api_server(api_state, guardian_node_state).await {
                tracing::error!("Guardian API server failed: {}", e);
            }
        }
    });

    // TODO: Remove naive sleep
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    info!("API servers started");

    // Validate cluster configuration
    let registry_client = bootstrap::MeasurementRegistryClient::new(registry_url.to_string());
    let bootstrap_nodes = registry_client.get_nodes(common::config::GUARDIAN_CERTIFICATE_NAMESPACE).await?;
    info!("Found {} nodes in registry", bootstrap_nodes.len());

    let api_address = format!("{}:{}", public_ip, common::config::GUARDIAN_API_PORT).parse()?;
    let raft_address = format!("{}:{}", public_ip, common::config::CONSENSUS_API_PORT).parse()?;

    let gossip_manager = bootstrap::PeerGossipManager::new(
        node_id,
        raft_address,
        api_address,
        instance_id.to_hex(),
        registry_url.to_string(),
        common::config::GUARDIAN_CERTIFICATE_NAMESPACE.to_string(),
    );

    info!("Sending initial advertisement to registry nodes...");
    gossip_manager.initial_advertise().await;

    tokio::spawn(async move { gossip_manager.run().await });

    // Create RAFT node
    let cluster_config = consensus::types::ClusterConfig {
        node_id,
        raft_address,
        api_address,
        measurement_registry_url: registry_url.to_string(),
        measurement_namespace: common::config::GUARDIAN_CERTIFICATE_NAMESPACE.to_string(),
    };

    let storage_path = std::path::PathBuf::from(common::config::RAFT_STORAGE_PATH);
    let raft_node = Arc::new(consensus::RaftNode::new(cluster_config, storage_path).await?);

    api_state.set_raft_node(raft_node.clone()).await;
    raft_api_state.set_raft_node(raft_node.clone()).await;

    // Start RAFT coordinator
    let coordinator = consensus::RaftCoordinator::new(node_id, raft_node);
    tokio::spawn(async move { coordinator.run().await });

    info!("Guardian initialization complete");
    info!("Waiting for peer attestation and cluster formation...");

    std::future::pending::<()>().await;
    Ok(())
}

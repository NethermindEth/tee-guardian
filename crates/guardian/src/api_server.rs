// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! API types and state for Guardian service.
//!
//! This module provides shared types used by the Guardian API:
//! - [`ApiState`] - Shared application state
//! - [`start_guardian_api_server`] - Start the combined API server
//! - Request/Response types for logging endpoints

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use consensus::RaftNode;

// Re-export ApiError from common (has IntoResponse via axum feature)
pub use common::ApiError;

/// Shared application state.
#[derive(Clone)]
pub struct ApiState {
    /// This node's ID (derived from IP address).
    pub node_id: u64,
    /// RAFT consensus node (None during bootstrap).
    pub raft_node: Arc<tokio::sync::RwLock<Option<Arc<RaftNode>>>>,
    /// Service start time for uptime tracking.
    pub start_time: std::time::Instant,
}

impl ApiState {
    /// Create new API state with node_id (RAFT node will be set later).
    pub fn new(node_id: u64) -> Self {
        Self { node_id, raft_node: Arc::new(tokio::sync::RwLock::new(None)), start_time: std::time::Instant::now() }
    }

    /// Set RAFT node after bootstrap/join completes.
    pub async fn set_raft_node(&self, raft_node: Arc<RaftNode>) {
        *self.raft_node.write().await = Some(raft_node);
    }
}

/// Set log context request.
#[derive(Debug, Serialize, Deserialize)]
pub struct SetLogContextRequest {
    /// Context key to set or remove.
    pub key: String,
    /// Context value (null or empty to remove the key).
    pub value: Option<String>,
}

/// Set log context response.
#[derive(Debug, Serialize, Deserialize)]
pub struct SetLogContextResponse {
    /// Operation status.
    pub status: String,
    /// Current log context after operation.
    pub context: std::collections::HashMap<String, String>,
}

/// Start the combined Guardian API server.
///
/// This starts a server with:
/// - `/metrics` - Prometheus metrics endpoint
/// - `/health` - Health check endpoint
/// - `/logging/set-context` - Dynamic log context management
/// - Guardian node routes (attestation, gossip) from bootstrap crate
///
/// # Errors
///
/// Returns an error if:
/// - Address parsing fails
/// - Port binding fails
/// - Server encounters a fatal error
pub async fn start_guardian_api_server(
    api_state: ApiState,
    guardian_node_state: bootstrap::GuardianNodeApiState,
) -> anyhow::Result<()> {
    // Initialize metrics (ignore errors if already initialized)
    let _ = common::metrics::init_metrics();
    let _ = consensus::metrics::init_raft_metrics();

    let addr: std::net::SocketAddr =
        format!("{}:{}", common::config::GUARDIAN_API_BIND_ADDR, common::config::GUARDIAN_API_PORT).parse()?;

    let app = create_router(api_state, guardian_node_state);

    info!("Starting Guardian API server on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

/// Create the combined Guardian API router.
fn create_router(api_state: ApiState, guardian_node_state: bootstrap::GuardianNodeApiState) -> Router {
    let guardian_router = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(health_handler))
        .route("/logging/set-context", post(set_log_context_handler))
        .with_state(api_state);

    guardian_router.merge(bootstrap::create_guardian_node_router(guardian_node_state)).layer(TraceLayer::new_for_http())
}

/// Prometheus metrics handler.
async fn metrics_handler() -> Response {
    let metrics = common::metrics::gather_metrics();
    (StatusCode::OK, [("content-type", "text/plain; version=0.0.4")], metrics).into_response()
}

/// Health check handler.
///
/// Returns node status including:
/// - Node ID
/// - RAFT status (or "BOOTSTRAPPING" if not yet initialized)
/// - Current term
/// - Cluster size
/// - Trusted peer count
async fn health_handler(State(state): State<ApiState>) -> Result<Json<common::HealthResponse>, StatusCode> {
    let trusted_peers = bootstrap::TrustedPeerRegistry::get().get_trusted_nodes().await;
    let raft_node_guard = state.raft_node.read().await;

    let (status, term, cluster_size) = match &*raft_node_guard {
        Some(raft_node) => {
            let node_state = raft_node.get_node_state().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let metrics = raft_node.raft().metrics().borrow().clone();
            let cluster_size = metrics.membership_config.membership().voter_ids().count();
            (node_state.status, node_state.term, cluster_size)
        }
        None => (common::NodeStatus::Bootstrapping, 0, 0),
    };

    Ok(Json(common::HealthResponse { node_id: state.node_id, status, term, cluster_size, trusted_peers }))
}

/// Handler for setting dynamic log context.
async fn set_log_context_handler(
    Json(req): Json<SetLogContextRequest>,
) -> Result<Json<SetLogContextResponse>, StatusCode> {
    common::logging::set_log_context(req.key.clone(), req.value).map_err(|_| StatusCode::BAD_REQUEST)?;

    Ok(Json(SetLogContextResponse { status: "ok".to_string(), context: common::logging::get_log_context() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_state_new() {
        let state = ApiState::new(123);
        assert_eq!(state.node_id, 123);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! RAFT HTTP API server for OpenRaft RPC.
//!
//! This module provides the HTTP endpoints required by OpenRaft's network layer:
//! - `/raft-append` - AppendEntries RPC
//! - `/raft-vote` - Vote RPC
//! - `/raft-snapshot` - InstallSnapshot RPC
//!
//! Gossip, peer cache, and other guardian-specific endpoints are in the bootstrap crate.

use axum::{extract::State, routing::post, Json, Router};
use common::ApiError;
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse, VoteRequest,
    VoteResponse,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::types::TypeConfig;
use crate::RaftNode;

/// State for the RAFT API server.
#[derive(Clone)]
pub struct RaftApiState {
    /// The RAFT node (set after bootstrap).
    pub raft_node: Arc<tokio::sync::RwLock<Option<Arc<RaftNode>>>>,
}

impl RaftApiState {
    /// Create a new state with no RAFT node.
    pub fn new() -> Self {
        Self { raft_node: Arc::new(tokio::sync::RwLock::new(None)) }
    }

    /// Set the RAFT node after bootstrap.
    pub async fn set_raft_node(&self, raft_node: Arc<RaftNode>) {
        *self.raft_node.write().await = Some(raft_node);
    }
}

impl Default for RaftApiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the RAFT API server.
pub async fn start_raft_api_server(state: RaftApiState) -> anyhow::Result<()> {
    let raft_api_bind_addr = common::config::CONSENSUS_API_BIND_ADDR;
    let raft_api_port = common::config::CONSENSUS_API_PORT;

    let addr: SocketAddr = format!("{}:{}", raft_api_bind_addr, raft_api_port).parse()?;

    let app = create_router(state);

    info!("Starting RAFT API server on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

macro_rules! require_raft {
    ($state:expr) => {{
        let guard = $state.raft_node.read().await;
        match &*guard {
            Some(node) => node.clone(),
            None => return Err(ApiError::Bootstrapping),
        }
    }};
}

fn create_router(state: RaftApiState) -> Router {
    Router::new()
        // RAFT RPC endpoints (used by OpenRaft network layer)
        .route("/raft-append", post(append_entries_handler))
        .route("/raft-vote", post(vote_handler))
        .route("/raft-snapshot", post(install_snapshot_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn append_entries_handler(
    State(state): State<RaftApiState>,
    Json(req): Json<AppendEntriesRequest<TypeConfig>>,
) -> Result<Json<AppendEntriesResponse<crate::NodeId>>, ApiError> {
    let raft_node = require_raft!(state);

    let resp = raft_node
        .raft()
        .append_entries(req)
        .await
        .map_err(|e| ApiError::Internal(format!("AppendEntries failed: {}", e)))?;

    Ok(Json(resp))
}

async fn vote_handler(
    State(state): State<RaftApiState>,
    Json(req): Json<VoteRequest<crate::NodeId>>,
) -> Result<Json<VoteResponse<crate::NodeId>>, ApiError> {
    let raft_node = require_raft!(state);

    let resp = raft_node.raft().vote(req).await.map_err(|e| ApiError::Internal(format!("Vote failed: {}", e)))?;

    Ok(Json(resp))
}

async fn install_snapshot_handler(
    State(state): State<RaftApiState>,
    Json(req): Json<InstallSnapshotRequest<TypeConfig>>,
) -> Result<Json<InstallSnapshotResponse<crate::NodeId>>, ApiError> {
    let raft_node = require_raft!(state);

    let resp = raft_node
        .raft()
        .install_snapshot(req)
        .await
        .map_err(|e| ApiError::Internal(format!("InstallSnapshot failed: {}", e)))?;

    Ok(Json(resp))
}

/// Expose router for testing.
#[cfg(test)]
pub fn test_router(state: RaftApiState) -> Router {
    create_router(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn test_raft_api_state_new() {
        let state = RaftApiState::new();
        // Should start with no raft node
        assert!(state.raft_node.try_read().unwrap().is_none());
    }

    #[test]
    fn test_api_error_bad_request() {
        let error = ApiError::BadRequest("test error".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_api_error_internal() {
        let error = ApiError::Internal("test error".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_api_error_bootstrapping() {
        let error = ApiError::Bootstrapping;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

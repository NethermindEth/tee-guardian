// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Public observability endpoints on isolated port.
//!
//! Provides Kubernetes-style health check endpoints:
//! - `/health` (liveness): Returns 200 if process is alive
//! - `/ready` (readiness): Returns 200 if ready to serve traffic

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Start the observability API server.
pub async fn start_observability_api(bind_addr: &str, port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{bind_addr}:{port}").parse()?;

    let app = Router::new().route("/health", get(health)).route("/ready", get(ready)).layer(TraceLayer::new_for_http());

    info!(
        addr = %addr,
        workers = common::config::OBSERVABILITY_API_WORKERS,
        "Starting observability API server"
    );

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Liveness probe: returns 200 if the process is running.
async fn health() -> StatusCode {
    StatusCode::OK
}

/// Readiness probe: returns 200 if ready to serve traffic.
///
/// TODO: Add actual readiness checks (e.g., RAFT cluster joined, etc.)
async fn ready() -> StatusCode {
    StatusCode::OK
}

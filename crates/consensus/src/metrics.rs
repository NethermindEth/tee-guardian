// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! RAFT-specific metrics for Guardian consensus
//!
//! This module provides metrics specific to RAFT consensus operations.
//! Base system metrics are in the common crate.

use lazy_static::lazy_static;
use prometheus::{HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts};
use std::sync::Once;

static INIT: Once = Once::new();

lazy_static! {

    // ========== RAFT Consensus Metrics ==========

    /// Total number of leader elections
    pub static ref RAFT_LEADER_ELECTIONS: IntCounter = IntCounter::new(
        "raft_leader_elections_total",
        "Total number of leader elections"
    ).unwrap();

    /// Current RAFT term
    pub static ref RAFT_TERM: IntGauge = IntGauge::new(
        "raft_term",
        "Current RAFT term number"
    ).unwrap();

    /// Current cluster size (number of voting nodes)
    pub static ref RAFT_CLUSTER_SIZE: IntGauge = IntGauge::new(
        "raft_cluster_size",
        "Number of voting nodes in the cluster"
    ).unwrap();

    /// Is this node the current leader
    pub static ref RAFT_IS_LEADER: IntGauge = IntGauge::new(
        "raft_is_leader",
        "Whether this node is currently the leader (1) or not (0)"
    ).unwrap();

    /// Log replication lag by target node
    pub static ref RAFT_LOG_REPLICATION_LAG: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "raft_log_replication_lag_entries",
            "Number of log entries this node is behind the leader"
        ),
        &["node_id"]
    ).unwrap();

    /// Heartbeat latency to other nodes (milliseconds)
    pub static ref RAFT_HEARTBEAT_LATENCY: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "raft_heartbeat_latency_ms",
            "Heartbeat round-trip latency to other nodes in milliseconds"
        )
        .buckets(vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0]),
        &["target_node"]
    ).unwrap();

    // ========== Security & Attestation Metrics ==========

    /// Total join requests received
    pub static ref JOIN_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "consensus_join_requests_total",
            "Total number of cluster join requests received"
        ),
        &["result"] // success, attestation_failed, measurement_untrusted, invalid_request, etc.
    ).unwrap();

    /// Measurement registry queries
    pub static ref MEASUREMENT_REGISTRY_QUERIES: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "measurement_registry_queries_total",
            "Total queries to measurement registry"
        ),
        &["result"] // trusted, not_found, error
    ).unwrap();

    /// Currently pending join requests (waiting for nonce verification)
    pub static ref PENDING_JOIN_REQUESTS: IntGauge = IntGauge::new(
        "pending_join_requests",
        "Number of join requests currently pending nonce verification"
    ).unwrap();

    // ========== Performance Metrics ==========

    /// Storage operation durations
    pub static ref STORAGE_OPERATION_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "storage_operation_duration_ms",
            "Duration of storage operations in milliseconds"
        )
        .buckets(vec![0.1, 0.5, 1.0, 5.0, 10.0, 50.0, 100.0]),
        &["operation"] // read, write, delete, snapshot
    ).unwrap();

    /// Total storage operations
    pub static ref STORAGE_OPERATIONS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "storage_operations_total",
            "Total number of storage operations"
        ),
        &["operation", "result"] // operation: read/write/delete, result: success/error
    ).unwrap();

    /// Network operations
    pub static ref NETWORK_OPERATIONS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "network_operations_total",
            "Total number of network operations"
        ),
        &["operation", "result"] // operation: send/receive, result: success/error
    ).unwrap();

    /// Active network connections
    pub static ref NETWORK_ACTIVE_CONNECTIONS: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "network_active_connections",
            "Number of active network connections"
        ),
        &["type"] // raft, api
    ).unwrap();

}

/// Initialize RAFT-specific metrics by registering them with the common registry
/// This function is idempotent and thread-safe - it only registers metrics once.
pub fn init_raft_metrics() -> Result<(), Box<dyn std::error::Error>> {
    INIT.call_once(|| {
        // Register RAFT metrics - panic on failure since this is critical setup
        common::metrics::METRICS_REGISTRY
            .register(Box::new(RAFT_LEADER_ELECTIONS.clone()))
            .expect("Failed to register raft_leader_elections_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(RAFT_TERM.clone()))
            .expect("Failed to register raft_term metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(RAFT_CLUSTER_SIZE.clone()))
            .expect("Failed to register raft_cluster_size metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(RAFT_IS_LEADER.clone()))
            .expect("Failed to register raft_is_leader metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(RAFT_LOG_REPLICATION_LAG.clone()))
            .expect("Failed to register raft_log_replication_lag_entries metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(RAFT_HEARTBEAT_LATENCY.clone()))
            .expect("Failed to register raft_heartbeat_latency_ms metric");

        // Register security metrics
        common::metrics::METRICS_REGISTRY
            .register(Box::new(JOIN_REQUESTS_TOTAL.clone()))
            .expect("Failed to register consensus_join_requests_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(MEASUREMENT_REGISTRY_QUERIES.clone()))
            .expect("Failed to register measurement_registry_queries_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(PENDING_JOIN_REQUESTS.clone()))
            .expect("Failed to register pending_join_requests metric");

        // Register performance metrics
        common::metrics::METRICS_REGISTRY
            .register(Box::new(STORAGE_OPERATION_DURATION.clone()))
            .expect("Failed to register storage_operation_duration_ms metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(STORAGE_OPERATIONS_TOTAL.clone()))
            .expect("Failed to register storage_operations_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(NETWORK_OPERATIONS_TOTAL.clone()))
            .expect("Failed to register network_operations_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(NETWORK_ACTIVE_CONNECTIONS.clone()))
            .expect("Failed to register network_active_connections metric");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_initialization() {
        // First call should succeed (or already initialized)
        let result = init_raft_metrics();
        assert!(result.is_ok());

        // Second call should also succeed (idempotent)
        let result2 = init_raft_metrics();
        assert!(result2.is_ok());
    }

    #[test]
    fn test_increment_counters() {
        let _ = common::metrics::init_metrics();
        let _ = init_raft_metrics();
        JOIN_REQUESTS_TOTAL.with_label_values(&["success"]).inc();
        MEASUREMENT_REGISTRY_QUERIES.with_label_values(&["trusted"]).inc();

        let output = common::metrics::gather_metrics();
        assert!(output.contains("consensus_join_requests_total"));
        assert!(output.contains("measurement_registry_queries_total"));
    }

    #[test]
    fn test_set_gauges() {
        let _ = init_raft_metrics();
        RAFT_TERM.set(42);
        RAFT_CLUSTER_SIZE.set(5);
        RAFT_IS_LEADER.set(1);

        let output = common::metrics::gather_metrics();
        assert!(output.contains("raft_term"));
        assert!(output.contains("raft_cluster_size"));
        assert!(output.contains("raft_is_leader"));
    }

    #[test]
    fn test_histogram_observe() {
        let _ = init_raft_metrics();
        RAFT_HEARTBEAT_LATENCY.with_label_values(&["test"]).observe(123.45);

        let output = common::metrics::gather_metrics();
        assert!(output.contains("raft_heartbeat_latency_ms"));
    }
}

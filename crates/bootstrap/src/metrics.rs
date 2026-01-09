// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Gossip protocol metrics for bootstrap service.
//!
//! These metrics track peer discovery and gossip protocol operations.

use lazy_static::lazy_static;
use prometheus::{IntCounter, IntCounterVec, IntGauge, Opts};
use std::sync::Once;

static INIT: Once = Once::new();

lazy_static! {
    // ========== Gossip Protocol Metrics ==========

    /// Number of peers in gossip cache
    pub static ref GOSSIP_PEERS_CACHED: IntGauge = IntGauge::new(
        "gossip_peers_cached",
        "Number of peers discovered via gossip protocol"
    ).unwrap();

    /// Total peer advertisements received
    pub static ref GOSSIP_ADVERTISEMENTS_RECEIVED: IntCounter = IntCounter::new(
        "gossip_advertisements_received_total",
        "Total peer advertisements received via gossip"
    ).unwrap();

    /// Peer advertisements rejected
    pub static ref GOSSIP_ADVERTISEMENTS_REJECTED: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "gossip_advertisements_rejected_total",
            "Total peer advertisements rejected"
        ),
        &["reason"] // invalid_signature, too_old, invalid_format, etc.
    ).unwrap();

    /// Peer advertisements sent
    pub static ref GOSSIP_ADVERTISEMENTS_SENT: IntCounter = IntCounter::new(
        "gossip_advertisements_sent_total",
        "Total peer advertisements sent to other nodes"
    ).unwrap();

    /// Peer list requests received
    pub static ref GOSSIP_PEER_LIST_REQUESTS: IntCounter = IntCounter::new(
        "gossip_peer_list_requests_total",
        "Total peer list requests received"
    ).unwrap();
}

/// Initialize gossip metrics by registering them with the common registry.
///
/// This function is idempotent and thread-safe - it only registers metrics once.
pub fn init_gossip_metrics() -> Result<(), Box<dyn std::error::Error>> {
    INIT.call_once(|| {
        common::metrics::METRICS_REGISTRY
            .register(Box::new(GOSSIP_PEERS_CACHED.clone()))
            .expect("Failed to register gossip_peers_cached metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(GOSSIP_ADVERTISEMENTS_RECEIVED.clone()))
            .expect("Failed to register gossip_advertisements_received_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(GOSSIP_ADVERTISEMENTS_REJECTED.clone()))
            .expect("Failed to register gossip_advertisements_rejected_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(GOSSIP_ADVERTISEMENTS_SENT.clone()))
            .expect("Failed to register gossip_advertisements_sent_total metric");
        common::metrics::METRICS_REGISTRY
            .register(Box::new(GOSSIP_PEER_LIST_REQUESTS.clone()))
            .expect("Failed to register gossip_peer_list_requests_total metric");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gossip_metrics_initialization() {
        let _ = common::metrics::init_metrics();
        let result = init_gossip_metrics();
        assert!(result.is_ok());

        // Second call should also succeed (idempotent)
        let result2 = init_gossip_metrics();
        assert!(result2.is_ok());
    }

    #[test]
    fn test_increment_gossip_counters() {
        let _ = common::metrics::init_metrics();
        let _ = init_gossip_metrics();

        GOSSIP_ADVERTISEMENTS_RECEIVED.inc();
        GOSSIP_ADVERTISEMENTS_SENT.inc();
        GOSSIP_PEER_LIST_REQUESTS.inc();
        GOSSIP_ADVERTISEMENTS_REJECTED.with_label_values(&["too_old"]).inc();
        GOSSIP_PEERS_CACHED.set(5);

        let output = common::metrics::gather_metrics();
        assert!(output.contains("gossip_advertisements_received_total"));
        assert!(output.contains("gossip_advertisements_sent_total"));
        assert!(output.contains("gossip_peer_list_requests_total"));
    }
}

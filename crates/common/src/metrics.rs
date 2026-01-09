// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Base metrics infrastructure for Guardian services
//!
//! Provides common Prometheus metrics and registry setup.
//! Individual services can extend with service-specific metrics.

use lazy_static::lazy_static;
use prometheus::{IntGauge, Registry};
use std::sync::Once;

static INIT: Once = Once::new();

lazy_static! {
    /// Global metrics registry
    pub static ref METRICS_REGISTRY: Registry = Registry::new();

    // ========== System Health Metrics ==========

    /// Node uptime in seconds
    pub static ref NODE_UPTIME_SECONDS: IntGauge = IntGauge::new(
        "node_uptime_seconds",
        "Number of seconds since node started"
    ).unwrap();

    /// Last successful health check timestamp
    pub static ref LAST_HEALTH_CHECK: IntGauge = IntGauge::new(
        "last_health_check_timestamp",
        "Unix timestamp of last successful health check"
    ).unwrap();
}

/// Initialize base metrics by registering them with the global registry
/// This function is idempotent and thread-safe - it only registers metrics once.
pub fn init_metrics() -> Result<(), Box<dyn std::error::Error>> {
    INIT.call_once(|| {
        // Panic on registration failure since this is a critical setup step
        METRICS_REGISTRY
            .register(Box::new(NODE_UPTIME_SECONDS.clone()))
            .expect("Failed to register node_uptime_seconds metric");
        METRICS_REGISTRY
            .register(Box::new(LAST_HEALTH_CHECK.clone()))
            .expect("Failed to register last_health_check_timestamp metric");
    });
    Ok(())
}

/// Get metrics in Prometheus text format
pub fn gather_metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let metric_families = METRICS_REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_initialization() {
        // First call should succeed (or already initialized)
        let result = init_metrics();
        assert!(result.is_ok());

        // Second call should also succeed (idempotent)
        let result2 = init_metrics();
        assert!(result2.is_ok());
    }

    #[test]
    fn test_gather_metrics() {
        let _ = init_metrics();
        let output = gather_metrics();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_set_gauges() {
        let _ = init_metrics();
        NODE_UPTIME_SECONDS.set(42);
        LAST_HEALTH_CHECK.set(1234567890);

        let output = gather_metrics();
        assert!(output.contains("node_uptime_seconds"));
        assert!(output.contains("last_health_check_timestamp"));
    }
}
